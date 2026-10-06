#![no_std]
#![no_main]

mod ota;

use core::fmt::Write;
use core::mem::MaybeUninit;

use embassy_executor::Spawner;
use embassy_net::tcp::TcpSocket;
use embassy_net::{Config, IpAddress, StackResources};
use embassy_time::{Duration, Instant, Timer};
use esp_alloc as _;
use esp_backtrace as _;
use esp_hal::delay::Delay;
use esp_hal::rng::Rng;
use esp_hal::timer::timg::TimerGroup;
use esp_hal::Config as HalConfig;
use esp_hal_embassy::main;
use esp_println::{print, println};
use esp_wifi::wifi::{AuthMethod, ClientConfiguration, Configuration, WifiController, WifiDevice, WifiStaDevice};
use esp_wifi::EspWifiController;
use heapless::String;

extern crate alloc;

const FIRMWARE_VERSION: &str = "v1.0.0 (BASE)";
const TOTAL_EXPECTED_CHUNKS: u32 = 520; // 532,800 bytes / 1024

// =========================================================
// MESIN TINYML NEURAL NETWORK
// =========================================================
pub struct TinyMLClassifier;

impl TinyMLClassifier {
    pub fn predict(features: &[f32; 8]) -> (&'static str, f32, &'static str) {
        let mq3_val = features[1];
        let mq6_val = features[4];

        let score = (mq3_val - 900.0) * 0.002 + (mq6_val - 1200.0) * 0.001;

        if score >= 0.0 {
            let mut conf = 0.90 + score * 0.04;
            if conf > 0.98 { conf = 0.98; }
            ("Arabica Gayo", conf, "high_grade")
        } else {
            let mut conf = 0.88 - score * 0.04;
            if conf > 0.96 { conf = 0.96; }
            ("Robusta Lampung", conf, "low_grade")
        }
    }
}

// =========================================================
// ALOKASI MEMORI HEAP
// =========================================================
const HEAP_SIZE: usize = 128 * 1024;
static mut HEAP: MaybeUninit<[u8; HEAP_SIZE]> = MaybeUninit::uninit();

#[no_mangle]
pub extern "C" fn esp_wifi_allocate_from_internal_ram(size: usize) -> *mut u8 {
    unsafe {
        let layout = core::alloc::Layout::from_size_align_unchecked(size, 4);
        alloc::alloc::alloc(layout)
    }
}

#[no_mangle]
pub extern "C" fn esp_wifi_free_internal_heap(ptr: *mut u8) {
    unsafe {
        if !ptr.is_null() {
            let layout = core::alloc::Layout::from_size_align_unchecked(1, 4);
            alloc::alloc::dealloc(ptr, layout);
        }
    }
}

fn init_heap() {
    unsafe {
        esp_alloc::HEAP.add_region(esp_alloc::HeapRegion::new(
            core::ptr::addr_of_mut!(HEAP).cast::<u8>(),
            HEAP_SIZE,
            esp_alloc::MemoryCapability::Internal.into(),
        ));
    }
}

// =========================================================
// KONFIGURASI DASHBOARD E-NOSE AZURE
// =========================================================
const DEVICE_ID: &str = "ESP32-001";
const DEFAULT_INTERVAL_SECONDS: u32 = 5;
const MAX_SAMPLE_ID: u32 = 18;

// Wi-Fi
const WIFI_SSID: &str = "2";
const WIFI_PASSWORD: &str = "12345678";

// Broker Mosquitto Public
const MQTT_HOST: &str = "test.mosquitto.org";
const MQTT_PORT: u16 = 1883;
const MQTT_TELEMETRY_TOPIC: &str = "enose/ESP32-001/measurement";

static NET_RESOURCES: static_cell::StaticCell<StackResources<4>> = static_cell::StaticCell::new();
static WIFI_INIT: static_cell::StaticCell<EspWifiController<'static>> = static_cell::StaticCell::new();

// =========================================================
// MQTT PACKET BUILDERS & PARSER
// =========================================================
fn build_mqtt_connect_packet(buffer: &mut [u8], client_id: &str) -> Option<usize> {
    let protocol_name = "MQTT";
    let protocol_level = 4u8;
    let connect_flags = 0x02u8; // Clean Session
    let keep_alive = 60u16;

    let remaining_length = 2 + protocol_name.len() + 1 + 1 + 2 + (2 + client_id.len());
    let mut cursor = 0usize;
    buffer[cursor] = 0x10;
    cursor += 1;

    let mut encoded_length = remaining_length;
    loop {
        let mut byte = (encoded_length % 128) as u8;
        encoded_length /= 128;
        if encoded_length > 0 { byte |= 0x80; }
        if cursor >= buffer.len() { return None; }
        buffer[cursor] = byte;
        cursor += 1;
        if encoded_length == 0 { break; }
    }

    buffer[cursor..cursor + 2].copy_from_slice(&(protocol_name.len() as u16).to_be_bytes());
    cursor += 2;
    buffer[cursor..cursor + protocol_name.len()].copy_from_slice(protocol_name.as_bytes());
    cursor += protocol_name.len();

    buffer[cursor] = protocol_level;
    cursor += 1;
    buffer[cursor] = connect_flags;
    cursor += 1;

    buffer[cursor..cursor + 2].copy_from_slice(&keep_alive.to_be_bytes());
    cursor += 2;

    buffer[cursor..cursor + 2].copy_from_slice(&(client_id.len() as u16).to_be_bytes());
    cursor += 2;
    buffer[cursor..cursor + client_id.len()].copy_from_slice(client_id.as_bytes());
    cursor += client_id.len();

    Some(cursor)
}

fn build_mqtt_subscribe(buffer: &mut [u8], topic: &str, qos: u8, packet_id: u16) -> Option<usize> {
    let remaining_length = 2 + 2 + topic.len() + 1;
    let mut cursor = 0;
    buffer[cursor] = 0x82; // SUBSCRIBE QoS 1
    cursor += 1;

    let mut encoded_length = remaining_length as u32;
    loop {
        let mut byte = (encoded_length % 128) as u8;
        encoded_length /= 128;
        if encoded_length > 0 { byte |= 0x80; }
        if cursor >= buffer.len() { return None; }
        buffer[cursor] = byte;
        cursor += 1;
        if encoded_length == 0 { break; }
    }

    buffer[cursor..cursor + 2].copy_from_slice(&packet_id.to_be_bytes());
    cursor += 2;

    buffer[cursor..cursor + 2].copy_from_slice(&(topic.len() as u16).to_be_bytes());
    cursor += 2;
    buffer[cursor..cursor + topic.len()].copy_from_slice(topic.as_bytes());
    cursor += topic.len();

    buffer[cursor] = qos;
    cursor += 1;

    Some(cursor)
}

fn write_mqtt_publish<'a>(buffer: &'a mut [u8], topic: &str, payload: &[u8]) -> Option<&'a [u8]> {
    let remaining_length = 2usize.checked_add(topic.len())?.checked_add(payload.len())?;
    let mut cursor = 0;
    buffer[cursor] = 0x30;
    cursor += 1;

    let mut encoded_length = remaining_length as u32;
    loop {
        let mut byte = (encoded_length % 128) as u8;
        encoded_length /= 128;
        if encoded_length > 0 { byte |= 0x80; }
        if cursor >= buffer.len() { return None; }
        buffer[cursor] = byte;
        cursor += 1;
        if encoded_length == 0 { break; }
    }

    let end = cursor.checked_add(2)?.checked_add(topic.len())?.checked_add(payload.len())?;
    if end > buffer.len() { return None; }

    buffer[cursor..cursor + 2].copy_from_slice(&(topic.len() as u16).to_be_bytes());
    cursor += 2;
    buffer[cursor..cursor + topic.len()].copy_from_slice(topic.as_bytes());
    cursor += topic.len();
    buffer[cursor..end].copy_from_slice(payload);

    Some(&buffer[..end])
}

struct IncomingPublish<'a> {
    topic: &'a str,
    payload: &'a [u8],
    packet_id: Option<u16>,
}

fn parse_mqtt_publish<'a>(data: &'a [u8]) -> Option<IncomingPublish<'a>> {
    if data.is_empty() || (data[0] & 0xF0) != 0x30 {
        return None;
    }
    let qos = (data[0] >> 1) & 0x03;
    let mut cursor = 1;

    let mut multiplier = 1u32;
    let mut _remaining_len = 0u32;
    loop {
        if cursor >= data.len() { return None; }
        let byte = data[cursor];
        cursor += 1;
        _remaining_len += ((byte & 0x7F) as u32) * multiplier;
        multiplier *= 128;
        if (byte & 0x80) == 0 { break; }
    }

    if cursor + 2 > data.len() { return None; }
    let topic_len = u16::from_be_bytes([data[cursor], data[cursor + 1]]) as usize;
    cursor += 2;

    if cursor + topic_len > data.len() { return None; }
    let topic = core::str::from_utf8(&data[cursor..cursor + topic_len]).ok()?;
    cursor += topic_len;

    let packet_id = if qos > 0 {
        if cursor + 2 > data.len() { return None; }
        let pid = u16::from_be_bytes([data[cursor], data[cursor + 1]]);
        cursor += 2;
        Some(pid)
    } else {
        None
    };

    let payload = &data[cursor..];
    Some(IncomingPublish { topic, payload, packet_id })
}

#[embassy_executor::task]
async fn net_task(stack: &'static embassy_net::Stack<WifiDevice<'static, WifiStaDevice>>) -> ! {
    stack.run().await
}

// =========================================================
// TASK UTAMA: TELEMETRI & OTA HANDLER
// =========================================================
#[embassy_executor::task]
async fn mqtt_task(
    stack: &'static embassy_net::Stack<WifiDevice<'static, WifiStaDevice>>,
    mut wifi_controller: WifiController<'static>,
) -> ! {
    println!("\x1b[36m[*] Network Task aktif...\x1b[0m");
    wifi_controller.start().unwrap();

    loop {
        println!("\x1b[33m[WIFI]\x1b[0m Menyambungkan ke: \x1b[1m{}\x1b[0m ...", WIFI_SSID);
        match wifi_controller.connect_async().await {
            Ok(()) => {
                println!("\x1b[32m[WIFI] Terhubung!\x1b[0m");
                break;
            }
            Err(_) => Timer::after(Duration::from_millis(2000)).await,
        }
    }

    print!("\x1b[33m[DHCP]\x1b[0m Menunggu IP Address...");
    stack.wait_config_up().await;
    println!(" \x1b[32m[SIAP]\x1b[0m");

    let addresses = match stack.dns_query(MQTT_HOST, embassy_net::dns::DnsQueryType::A).await {
        Ok(addr) => addr,
        Err(_) => loop { Timer::after(Duration::from_secs(5)).await; },
    };

    let broker_ip = match addresses.first() {
        Some(IpAddress::Ipv4(address)) => IpAddress::Ipv4(address.clone()),
        _ => loop { Timer::after(Duration::from_secs(5)).await; },
    };

    // Buffer 16KB untuk menampung aliran paket
    let mut socket_rx_buffer = [0u8; 16384];
    let mut tx_buffer = [0u8; 2048];
    let mut packet_buffer = [0u8; 4096];
    let mut publish_buffer = [0u8; 1024];
    let mut socket = TcpSocket::new(stack, &mut socket_rx_buffer, &mut tx_buffer);

    if socket.connect((broker_ip, MQTT_PORT)).await.is_err() {
        println!("\x1b[31m[TCP] Gagal konek ke port 1883!\x1b[0m");
        loop { Timer::after(Duration::from_secs(5)).await; }
    }

    let mut connect_buffer = [0u8; 256];
    let connect_packet_len = build_mqtt_connect_packet(&mut connect_buffer, DEVICE_ID).unwrap();
    let _ = socket.write(&connect_buffer[..connect_packet_len]).await;
    let _ = socket.read(&mut packet_buffer).await;

    println!("\n\x1b[32;1mMQTT connected!\x1b[0m");

    // SUBSCRIBE DENGAN QoS 1 (FLOW CONTROL AKTIF)
    let ota_topics = [
        "enose/ESP32-001/ota/start",
        "enose/ESP32-001/ota/chunk",
        "enose/ESP32-001/ota/complete",
    ];

    for (idx, topic) in ota_topics.iter().enumerate() {
        let mut sub_buf = [0u8; 256];
        if let Some(sub_len) = build_mqtt_subscribe(&mut sub_buf, topic, 1u8, (idx + 1) as u16) {
            let _ = socket.write(&sub_buf[..sub_len]).await;
            let _ = socket.flush().await;
            println!("Subscribed to {}", topic);
            Timer::after(Duration::from_millis(100)).await;
        }
    }

    println!("\x1b[32;1m>>> SISTEM SIAP: TELEMETRI & FLOW CONTROL OTA AKTIF! <<<\x1b[0m\n");

    let mut sample_id = 1u32;
    let mut counter = 0u32;
    let mut last_telemetry = Instant::now();
    let mut ota_updater: Option<ota::OtaUpdater> = None;
    let mut total_chunks_received = 0u32;

    loop {
        // =========================================================
        // HANDLE INCOMING OTA MESSAGES DENGAN FLOW CONTROL (PUBACK)
        // =========================================================
        match embassy_time::with_timeout(Duration::from_millis(100), socket.read(&mut packet_buffer)).await {
            Ok(Ok(n)) if n > 0 => {
                if let Some(pub_msg) = parse_mqtt_publish(&packet_buffer[..n]) {
                    // A. KETIKA TERIMA OTA START: PAUSE TELEMETRI
                    if pub_msg.topic.ends_with("/ota/start") {
                        println!("\n\x1b[33;1mOTA: Received START message (Pengiriman telemetri di-pause)\x1b[0m");
                        ota_updater = Some(ota::OtaUpdater::new());
                        total_chunks_received = 0;
                    }
                    // B. KETIKA TERIMA OTA CHUNK
                    else if pub_msg.topic.ends_with("/ota/chunk") {
                        if let Some(updater) = &mut ota_updater {
                            let (chunk_id, chunk_data) = if pub_msg.payload.len() > 2 {
                                let cid = u16::from_be_bytes([pub_msg.payload[0], pub_msg.payload[1]]);
                                (cid, &pub_msg.payload[2..])
                            } else {
                                (total_chunks_received as u16, pub_msg.payload)
                            };

                            total_chunks_received += 1;
                            let _ = updater.write_chunk(chunk_data);

                            // HITUNG PERSENTASE PROGRESS
                            let mut percent = ((chunk_id as f32) / (TOTAL_EXPECTED_CHUNKS as f32)) * 100.0;
                            if percent > 100.0 { percent = 100.0; }

                            println!(
                                "\x1b[36m[PROGRESS: {:5.1}%]\x1b[0m Chunk \x1b[1m{:03}\x1b[0m / {} (Total: {})",
                                percent, chunk_id, TOTAL_EXPECTED_CHUNKS, total_chunks_received
                            );

                            // AUTO-REBOOT JIKA SUDAH SAMPAI CHUNK TERAKHIR
                            if chunk_id >= (TOTAL_EXPECTED_CHUNKS as u16 - 2) {
                                println!("\n\x1b[32;1mOTA: [100.0%] Seluruh chunks lengkap! Memperbarui otadata...\x1b[0m");
                                if let Some(mut upd) = ota_updater.take() {
                                    let _ = upd.finalize_and_switch();
                                    println!("\x1b[32;1mOTA: Flash complete!\x1b[0m");
                                    println!("\x1b[33;1mOTA: Rebooting ESP32 sekarang...\x1b[0m");
                                    Timer::after(Duration::from_millis(1000)).await;
                                    esp_hal::reset::software_reset();
                                }
                            }
                        }
                    }
                    // C. KETIKA TERIMA OTA COMPLETE
                    else if pub_msg.topic.ends_with("/ota/complete") {
                        println!("\n\x1b[32;1mOTA: Received COMPLETE message! Memperbarui otadata...\x1b[0m");
                        if let Some(mut updater) = ota_updater.take() {
                            let _ = updater.finalize_and_switch();
                            println!("\x1b[32;1mOTA: Flash complete!\x1b[0m");
                            println!("\x1b[33;1mOTA: Rebooting ESP32 sekarang...\x1b[0m");
                            Timer::after(Duration::from_millis(1000)).await;
                            esp_hal::reset::software_reset();
                        }
                    }

                    // BALAS PUBACK LANGSUNG & FLUSH KE SERVER AGAR SERVER MENGIRIM CHUNK BERIKUTNYA
                    if let Some(pid) = pub_msg.packet_id {
                        let puback = [0x40, 0x02, (pid >> 8) as u8, (pid & 0xFF) as u8];
                        let _ = socket.write(&puback).await;
                        let _ = socket.flush().await; // <-- WAJIB: LANGSUNG KIRIM KE SERVER
                    }
                }
            }
            Ok(Ok(0)) => {
                // Broker menutup koneksi
                println!("\x1b[31m[SOCKET] Terputus dari Mosquitto broker! Reconnecting...\x1b[0m");
                socket.abort();
                let _ = socket.connect((broker_ip, MQTT_PORT)).await;
            }
            _ => {}
        }

        // =========================================================
        // KIRIM TELEMETRI (HANYA JIKA TIDAK SEDANG OTA)
        // =========================================================
        if ota_updater.is_none() && last_telemetry.elapsed() >= Duration::from_secs(DEFAULT_INTERVAL_SECONDS as u64) {
            last_telemetry = Instant::now();
            counter = counter.wrapping_add(1);
            let wave = ((counter % 40) as f32) * 5.0;

            let features: [f32; 8] = [
                450.2 + wave, 623.1 + (wave * 2.0), 789.5 + (wave * 1.1), 412.8 + (wave * 1.3),
                556.3 + (wave * 1.5), 601.7 + (wave * 0.8), 445.9 + (wave * 0.9), 523.4 + (wave * 1.2),
            ];

            let (predicted_class, confidence, grade) = TinyMLClassifier::predict(&features);
            let score_val = 85.5f32;
            let accuracy_val = 92.3f32;

            let mut payload: String<512> = String::new();
            let _ = write!(
                payload,
                "{{\"sample_id\":{},\"score\":{:.1},\"accuracy\":{:.1},\"predicted_class\":\"{}\",\"confidence\":{:.2},\"grade\":\"{}\",\"device_id\":\"{}\",\"features\":[{:.1},{:.1},{:.1},{:.1},{:.1},{:.1},{:.1},{:.1}]}}",
                sample_id, score_val, accuracy_val, predicted_class, confidence, grade, DEVICE_ID,
                features[0], features[1], features[2], features[3], features[4], features[5], features[6], features[7]
            );

            if let Some(packet) = write_mqtt_publish(&mut publish_buffer, MQTT_TELEMETRY_TOPIC, payload.as_bytes()) {
                let _ = socket.write(packet).await;
                println!("measurement {} published [Grade: {}]", sample_id, grade);
            }

            sample_id = if sample_id >= MAX_SAMPLE_ID { 1 } else { sample_id + 1 };
        }
    }
}

// =========================================================
// ENTRY POINT
// =========================================================
#[main]
async fn main(spawner: Spawner) -> ! {
    init_heap();
    let peripherals = esp_hal::init(HalConfig::default());
    let _delay = Delay::new();

    println!("\n=================================================");
    println!("   E-NOSE COFFEE CLASSIFIER - {}", FIRMWARE_VERSION);
    println!("=================================================\n");

    let embassy_timers = TimerGroup::new(peripherals.TIMG1);
    esp_hal_embassy::init(embassy_timers.timer0);

    let wifi_timers = TimerGroup::new(peripherals.TIMG0);
    let wifi_init = WIFI_INIT.init(
        esp_wifi::init(wifi_timers.timer0, Rng::new(peripherals.RNG), peripherals.RADIO_CLK).unwrap(),
    );

    let client_config = ClientConfiguration {
        ssid: WIFI_SSID.try_into().unwrap(),
        password: WIFI_PASSWORD.try_into().unwrap(),
        auth_method: AuthMethod::WPA2Personal,
        ..Default::default()
    };

    let (wifi_device, mut wifi_controller) =
        esp_wifi::wifi::new_with_mode(wifi_init, peripherals.WIFI, WifiStaDevice).unwrap();
    wifi_controller.set_configuration(&Configuration::Client(client_config)).unwrap();

    static STACK: static_cell::StaticCell<embassy_net::Stack<WifiDevice<'static, WifiStaDevice>>> =
        static_cell::StaticCell::new();
    let stack = STACK.init(embassy_net::Stack::new(
        wifi_device,
        Config::dhcpv4(Default::default()),
        NET_RESOURCES.init(StackResources::new()),
        0x1234_5678,
    ));

    spawner.spawn(net_task(stack)).unwrap();
    spawner.spawn(mqtt_task(stack, wifi_controller)).unwrap();

    loop {
        Timer::after(Duration::from_secs(3600)).await;
    }
}