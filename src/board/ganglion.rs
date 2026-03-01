//! Ganglion board — 4-channel EEG via native Bluetooth LE.
//!
//! Uses the [`btleplug`] crate to communicate directly with the Ganglion's
//! BLE radio.  No USB dongle or proprietary BGLIB library required.
//!
//! ## BLE characteristics
//! - **Write**: `2d30c083-f39f-4ce6-923f-3484ea480596`
//! - **Notify**: `2d30c082-f39f-4ce6-923f-3484ea480596`
//! - **Software revision**: `00002a28-0000-1000-8000-00805f9b34fb` (firmware version)

#[cfg(feature = "ble")]
mod impl_ {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    use btleplug::api::{Central, Manager as _, Peripheral as _, ScanFilter, WriteType, Characteristic};
    use btleplug::platform::{Adapter, Manager, Peripheral};
    use futures::StreamExt;

    use crate::board::Board;
    use crate::electrode::ElectrodeLayout;
    use crate::error::{OpenBciError, Result};
    use crate::packet::{decode_ganglion, GanglionState};
    use crate::sample::{Sample, StreamHandle};

    // ── BLE UUIDs ─────────────────────────────────────────────────────────────

    const WRITE_CHAR_UUID:  &str = "2d30c083-f39f-4ce6-923f-3484ea480596";
    const NOTIFY_CHAR_UUID: &str = "2d30c082-f39f-4ce6-923f-3484ea480596";
    const SW_REVISION_UUID: &str = "00002a28-0000-1000-8000-00805f9b34fb";

    /// Compare a BLE characteristic's UUID against a UUID string (case-insensitive).
    fn uuid_eq(c: &Characteristic, uuid_str: &str) -> bool {
        c.uuid.to_string().eq_ignore_ascii_case(uuid_str)
    }

    // ─────────────────────────────────────────────────────────────────────────

    /// How to identify the Ganglion during BLE scanning.
    #[derive(Debug, Clone, Default)]
    pub struct GanglionFilter {
        /// If set, only connect to this exact hardware address.
        pub mac_address: Option<String>,
        /// If set, only match peripherals whose name contains this substring
        /// (case-insensitive).  Defaults to matching "Ganglion" or "Simblee".
        pub device_name: Option<String>,
    }

    /// Which firmware version the Ganglion is running.
    #[derive(Debug, Clone, Default)]
    pub enum GanglionFirmware {
        /// Read the software revision characteristic to determine the version.
        #[default]
        Auto,
        V2,
        V3,
    }

    /// Configuration for [`GanglionBoard`].
    #[derive(Debug, Clone)]
    pub struct GanglionConfig {
        /// How long to scan for the device before giving up.
        pub scan_timeout: Duration,
        pub firmware:     GanglionFirmware,
        pub filter:       GanglionFilter,
    }

    impl Default for GanglionConfig {
        fn default() -> Self {
            Self {
                scan_timeout: Duration::from_secs(10),
                firmware:     GanglionFirmware::Auto,
                filter:       GanglionFilter::default(),
            }
        }
    }

    // ─────────────────────────────────────────────────────────────────────────

    /// Ganglion 4-channel BLE board.
    ///
    /// # Example
    /// ```rust,no_run
    /// use openbci::board::ganglion::{GanglionBoard, GanglionConfig};
    /// use openbci::board::Board;
    /// use openbci::electrode::ElectrodeLayout;
    ///
    /// let mut board = GanglionBoard::new(GanglionConfig::default())
    ///     .with_electrode_layout(ElectrodeLayout::from_labels(&["Fp1","Fp2","C3","C4"]));
    ///
    /// board.prepare().unwrap();
    /// let stream = board.start_stream().unwrap();
    /// for sample in stream.into_iter().take(200) {
    ///     println!("{:?}", sample.eeg);
    /// }
    /// board.release().unwrap();
    /// ```
    pub struct GanglionBoard {
        config:           GanglionConfig,
        electrode_layout: ElectrodeLayout,
        runtime:          tokio::runtime::Runtime,
        peripheral:       Option<Peripheral>,
        write_char:       Option<Characteristic>,
        notify_char:      Option<Characteristic>,
        firmware:         u8,
        streaming:        bool,
        keep_alive:       Arc<AtomicBool>,
    }

    impl GanglionBoard {
        /// Create a new Ganglion BLE board driver.
        ///
        /// A multi-threaded Tokio runtime is created internally to drive the
        /// `btleplug` async BLE stack.  The runtime is owned by this struct and
        /// lives until [`release`](crate::board::Board::release) is called.
        pub fn new(config: GanglionConfig) -> Self {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .expect("Failed to create tokio runtime");

            Self {
                config,
                electrode_layout: ElectrodeLayout::new(4),
                runtime,
                peripheral:  None,
                write_char:  None,
                notify_char: None,
                firmware:    3,
                streaming:   false,
                keep_alive:  Arc::new(AtomicBool::new(false)),
            }
        }

        /// Builder: attach an electrode layout describing the 4 channel sites.
        pub fn with_electrode_layout(mut self, layout: ElectrodeLayout) -> Self {
            self.electrode_layout = layout;
            self
        }

        /// Scan the BLE adapter for a Ganglion peripheral matching `filter`.
        ///
        /// Polls the adapter's peripheral list every 200 ms until a matching
        /// device is found or `timeout` elapses.  Returns `None` on timeout.
        async fn scan_for_ganglion(
            adapter: &Adapter,
            filter:  &GanglionFilter,
            timeout: Duration,
        ) -> Option<Peripheral> {
            adapter.start_scan(ScanFilter::default()).await.ok()?;
            let deadline = tokio::time::Instant::now() + timeout;

            loop {
                if tokio::time::Instant::now() > deadline { break; }

                if let Ok(peripherals) = adapter.peripherals().await {
                    for p in peripherals {
                        if let Ok(Some(props)) = p.properties().await {
                            let matched = if let Some(ref mac) = filter.mac_address {
                                p.address().to_string().eq_ignore_ascii_case(mac)
                            } else if let Some(ref name) = props.local_name {
                                let lc = name.to_lowercase();
                                if let Some(ref target) = filter.device_name {
                                    lc.contains(&target.to_lowercase())
                                } else {
                                    lc.contains("ganglion") || lc.contains("simblee")
                                }
                            } else {
                                false
                            };

                            if matched {
                                adapter.stop_scan().await.ok();
                                return Some(p);
                            }
                        }
                    }
                }
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
            adapter.stop_scan().await.ok();
            None
        }

        /// Write `data` to the Ganglion's BLE write characteristic
        /// (`WriteType::WithoutResponse`) using the internal Tokio runtime.
        fn write_ble(&self, data: &[u8]) -> Result<()> {
            let p  = self.peripheral.as_ref().ok_or(OpenBciError::BoardNotPrepared)?;
            let ch = self.write_char.as_ref().ok_or(OpenBciError::BoardNotPrepared)?;
            let data = data.to_vec();
            let p_clone  = p.clone();
            let ch_clone = ch.clone();
            self.runtime.block_on(async move {
                p_clone.write(&ch_clone, &data, WriteType::WithoutResponse).await
            })?;
            Ok(())
        }
    }

    // ─── Board trait ──────────────────────────────────────────────────────────

    impl Board for GanglionBoard {
        fn prepare(&mut self) -> Result<()> {
            if self.peripheral.is_some() { return Ok(()); }

            let timeout = self.config.scan_timeout;
            let filter  = self.config.filter.clone();

            let (_adapter, peripheral) = self.runtime.block_on(async {
                let manager  = Manager::new().await?;
                let adapters = manager.adapters().await?;
                let adapter  = adapters.into_iter().next().ok_or_else(|| {
                    OpenBciError::BoardNotReady("No BLE adapter found".into())
                })?;
                let peripheral = Self::scan_for_ganglion(&adapter, &filter, timeout).await
                    .ok_or_else(|| OpenBciError::BoardNotReady("Ganglion not found".into()))?;
                Ok::<_, OpenBciError>((adapter, peripheral))
            })?;

            // Connect with retries
            let mut ok = false;
            for attempt in 1..=3 {
                if self.runtime.block_on(peripheral.connect()).is_ok() { ok = true; break; }
                log::warn!("BLE connect attempt {}/3 failed", attempt);
                std::thread::sleep(Duration::from_secs(1));
            }
            if !ok {
                return Err(OpenBciError::BoardNotReady("Could not connect to Ganglion".into()));
            }

            self.runtime.block_on(peripheral.discover_services())?;

            let chars = peripheral.characteristics();

            let write_char = chars.iter().find(|c| uuid_eq(c, WRITE_CHAR_UUID)).cloned()
                .ok_or_else(|| OpenBciError::BoardNotReady("Write char not found".into()))?;
            let notify_char = chars.iter().find(|c| uuid_eq(c, NOTIFY_CHAR_UUID)).cloned()
                .ok_or_else(|| OpenBciError::BoardNotReady("Notify char not found".into()))?;

            // Firmware autodetect
            let firmware = match self.config.firmware {
                GanglionFirmware::V2   => 2,
                GanglionFirmware::V3   => 3,
                GanglionFirmware::Auto => {
                    if let Some(sw_char) = chars.iter().find(|c| uuid_eq(c, SW_REVISION_UUID)) {
                        let data = self.runtime
                            .block_on(peripheral.read(sw_char))
                            .unwrap_or_default();
                        if data.first().copied() == Some(b'3') { 3 } else { 2 }
                    } else {
                        3
                    }
                }
            };
            log::info!("Ganglion firmware: {}", firmware);

            self.runtime.block_on(peripheral.subscribe(&notify_char))?;

            self.firmware    = firmware;
            self.write_char  = Some(write_char);
            self.notify_char = Some(notify_char);
            self.peripheral  = Some(peripheral);
            Ok(())
        }

        fn start_stream(&mut self) -> Result<StreamHandle> {
            if self.streaming { return Err(OpenBciError::AlreadyStreaming); }

            self.write_ble(b"b")?;

            let peripheral = self.peripheral.clone()
                .ok_or(OpenBciError::BoardNotPrepared)?;
            let firmware   = self.firmware;

            let (sample_tx, sample_rx) = std::sync::mpsc::sync_channel::<Sample>(512);
            let (stop_tx, stop_rx)     = std::sync::mpsc::sync_channel::<()>(1);

            let keep_alive = self.keep_alive.clone();
            keep_alive.store(true, Ordering::Release);

            let tx = sample_tx.clone();
            self.runtime.spawn(async move {
                let mut notif_stream = match peripheral.notifications().await {
                    Ok(s) => s,
                    Err(e) => { log::error!("notifications() failed: {}", e); return; }
                };
                let mut state = GanglionState::default();

                loop {
                    if stop_rx.try_recv().is_ok() || !keep_alive.load(Ordering::Acquire) { break; }

                    match tokio::time::timeout(
                        Duration::from_millis(50),
                        notif_stream.next(),
                    ).await {
                        Ok(Some(n)) => {
                            for s in decode_ganglion(&n.value, &mut state, firmware) {
                                if tx.send(s).is_err() { return; }
                            }
                        }
                        Ok(None) => break,
                        Err(_)   => continue, // timeout → loop back and check stop
                    }
                }
            });

            self.streaming = true;
            Ok(StreamHandle { receiver: sample_rx, stop_tx: Some(stop_tx) })
        }

        fn stop_stream(&mut self) -> Result<()> {
            if !self.streaming { return Err(OpenBciError::NotStreaming); }
            self.keep_alive.store(false, Ordering::Release);
            let _ = self.write_ble(b"s");
            self.streaming = false;
            Ok(())
        }

        fn release(&mut self) -> Result<()> {
            if self.streaming { let _ = self.stop_stream(); }
            if let Some(ref p) = self.peripheral {
                let p = p.clone();
                let _ = self.runtime.block_on(p.disconnect());
            }
            self.peripheral  = None;
            self.write_char  = None;
            self.notify_char = None;
            Ok(())
        }

        fn send_command(&mut self, cmd: &str) -> Result<String> {
            self.write_ble(cmd.as_bytes())?;
            Ok(String::new())
        }

        fn electrode_layout(&self) -> &ElectrodeLayout        { &self.electrode_layout }
        fn set_electrode_layout(&mut self, l: ElectrodeLayout) { self.electrode_layout = l; }
        fn channel_count(&self) -> usize                       { 4 }
        fn sampling_rate(&self) -> u32                         { 200 }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Public re-exports
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(feature = "ble")]
pub use impl_::{GanglionBoard, GanglionConfig, GanglionFirmware, GanglionFilter};

#[cfg(not(feature = "ble"))]
compile_error!(
    "GanglionBoard requires the `ble` feature.  \
     Enable it in Cargo.toml, or use GanglionWifiBoard instead."
);
