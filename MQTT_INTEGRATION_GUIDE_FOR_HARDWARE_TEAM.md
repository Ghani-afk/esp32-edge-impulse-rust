# MQTT INTEGRATION GUIDE - Hardware Team (Bagian A)

## 📋 OVERVIEW

Dokumen ini adalah panduan untuk tim hardware (Bagian A) agar ESP32 bisa mengirim data sensor ke backend cloud (Bagian C) dengan benar.

**Tujuan:** ESP32 publish data sensor → MQTT Broker → Railway Backend → PostgreSQL Database

---

## 🔧 MQTT CONFIGURATION (WAJIB!)

### **Broker Details**
```rust
MQTT_BROKER = "broker.emqx.io"
MQTT_PORT   = 1883
MQTT_QOS    = 1  // At least once delivery
MQTT_TLS    = false  // Tidak pakai TLS/SSL
```

⚠️ **CRITICAL:** Broker harus **EXACTLY** `broker.emqx.io` port `1883`. Jangan pakai broker lain (test.mosquitto.org, localhost, dll)!

---

## 📡 MQTT TOPIC FORMAT

### **Topic Pattern (WAJIB!)**
```
enose/{DEVICE_ID}/measurement
```

### **Contoh Valid:**
```
✅ enose/ESP32-001/measurement
✅ enose/ESP32-JAKARTA-01/measurement
✅ enose/ESP32-BANDUNG-01/measurement
✅ enose/ESP32-LAB-05/measurement
```

### **Contoh SALAH:**
```
❌ sensor/ESP32-001/data           // Prefix salah
❌ ESP32-001/measurement           // Kurang prefix "enose/"
❌ enose/measurement               // Kurang device_id
❌ enose/ESP32-001/sensor          // Suffix salah (harus "measurement")
```

⚠️ **PENTING:** Backend subscribe ke `enose/+/measurement` (wildcard `+` = device_id apapun)

---

## 📦 JSON PAYLOAD FORMAT

### **Structure (WAJIB!)**
```json
{
  "device_id": "ESP32-001",
  "timestamp": "2026-09-23T10:30:00Z",
  "mq2": 450,
  "mq3": 320,
  "mq4": 280,
  "mq5": 390,
  "mq6": 310,
  "mq7": 270,
  "mq8": 340,
  "mq135": 410,
  "prediction": "Arabica Gayo",
  "confidence": 0.95
}
```

### **Field Specifications:**

| Field | Type | Required | Example | Notes |
|-------|------|----------|---------|-------|
| `device_id` | String | ✅ Yes | `"ESP32-001"` | Harus sama dengan device_id di topic |
| `timestamp` | String (ISO 8601) | ✅ Yes | `"2026-09-23T10:30:00Z"` | UTC atau WIB dengan timezone |
| `mq2` | Integer/Float | ✅ Yes | `450` | Sensor MQ-2 value |
| `mq3` | Integer/Float | ✅ Yes | `320` | Sensor MQ-3 value |
| `mq4` | Integer/Float | ✅ Yes | `280` | Sensor MQ-4 value |
| `mq5` | Integer/Float | ✅ Yes | `390` | Sensor MQ-5 value |
| `mq6` | Integer/Float | ✅ Yes | `310` | Sensor MQ-6 value |
| `mq7` | Integer/Float | ✅ Yes | `270` | Sensor MQ-7 value |
| `mq8` | Integer/Float | ✅ Yes | `340` | Sensor MQ-8 value |
| `mq135` | Integer/Float | ✅ Yes | `410` | Sensor MQ-135 value |
| `prediction` | String | ⚠️ Optional | `"Arabica Gayo"` | ML prediction result |
| `confidence` | Float (0.0-1.0) | ⚠️ Optional | `0.95` | ML confidence score |

### **Timestamp Format (ISO 8601):**

```rust
// ✅ BENAR
"2026-09-23T10:30:00Z"           // UTC (Zulu time)
"2026-09-23T17:30:00+07:00"      // WIB (UTC+7)
"2026-09-23T10:30:00.123Z"       // Dengan milliseconds

// ❌ SALAH
"23/09/2026 10:30:00"            // Format Indonesia
"2026-09-23 10:30:00"            // Kurang 'T' separator
1727078400                       // Unix timestamp (integer)
"10:30:00"                       // Hanya jam, tanpa tanggal
```

### **Data Type Rules:**

```json
// ✅ BENAR - Numbers tanpa quotes
{
  "mq2": 450,
  "mq3": 320.5,
  "confidence": 0.95
}

// ❌ SALAH - Numbers dalam string
{
  "mq2": "450",        // Jangan pakai quotes!
  "mq3": "320",
  "confidence": "0.95"
}

// ❌ SALAH - Invalid values
{
  "mq2": null,         // Null tidak valid
  "mq3": NaN,          // Not a Number
  "mq4": undefined     // Undefined
}
```

---

## 🔄 PUBLISH FLOW

### **Recommended Flow:**

```rust
loop {
    // 1. Baca sensor (8 sensors)
    let mq2_val = read_sensor_mq2();
    let mq3_val = read_sensor_mq3();
    // ... sampai mq135
    
    // 2. ML Inference (Edge Impulse)
    let (prediction, confidence) = run_ml_model([mq2_val, mq3_val, ...]);
    
    // 3. Build JSON payload
    let payload = json!({
        "device_id": DEVICE_ID,
        "timestamp": get_iso8601_timestamp(),
        "mq2": mq2_val,
        "mq3": mq3_val,
        "mq4": mq4_val,
        "mq5": mq5_val,
        "mq6": mq6_val,
        "mq7": mq7_val,
        "mq8": mq8_val,
        "mq135": mq135_val,
        "prediction": prediction,
        "confidence": confidence
    });
    
    // 4. Publish MQTT
    let topic = format!("enose/{}/measurement", DEVICE_ID);
    mqtt_client.publish(&topic, QoS::AtLeastOnce, false, payload.to_string())?;
    
    // 5. Delay
    delay_ms(1000); // 1 second interval
}
```

### **Publish Rate Limits:**

```
✅ RECOMMENDED:
   - 1 message per second (1 Hz)
   - 1 message per 5 seconds (0.2 Hz)

⚠️ WARNING:
   - 10 messages per second (10 Hz) → Bisa overload broker

❌ FORBIDDEN:
   - 100 messages per second (100 Hz) → Pasti di-ban!
```

---

## 🛡️ ERROR HANDLING

### **WiFi Reconnection:**

```rust
// ✅ BENAR - Cek WiFi sebelum publish
if !wifi_connected() {
    log::warn!("WiFi disconnected, reconnecting...");
    reconnect_wifi()?;
}

mqtt_publish(payload)?;
```

### **MQTT Connection Check:**

```rust
// ✅ BENAR - Handle connection loss
match mqtt_client.publish(topic, qos, retain, payload) {
    Ok(_) => {
        log::info!("Data published successfully");
    }
    Err(e) => {
        log::error!("MQTT publish failed: {:?}", e);
        reconnect_mqtt()?;
    }
}
```

### **Sensor Read Validation:**

```rust
// ✅ BENAR - Validate sensor values
let mq2_val = read_sensor_mq2();
if mq2_val < 0 || mq2_val > 4096 {
    log::warn!("Invalid MQ2 value: {}", mq2_val);
    return; // Skip publish untuk data invalid
}
```

---

## 🧪 TESTING & VALIDATION

### **Step 1: Serial Monitor Check**

Pastikan ESP32 menampilkan log seperti ini:

```
[INFO] WiFi connecting to: YourSSID
[INFO] WiFi connected! IP: 192.168.1.100
[INFO] MQTT connecting to broker.emqx.io:1883
[INFO] MQTT connected!
[INFO] [MQTT PUBLISHED] Tick: 1s
[INFO] [MQTT PUBLISHED] Tick: 2s
[INFO] [MQTT PUBLISHED] Tick: 3s
```

⚠️ Jika stuck di "MQTT connecting..." → cek broker address & port!

### **Step 2: Test Payload dengan MQTT Explorer**

1. Download MQTT Explorer: http://mqtt-explorer.com/
2. Connect ke `broker.emqx.io:1883`
3. Subscribe ke topic: `enose/#` (all topics)
4. Flash ESP32 dan cek apakah message muncul di MQTT Explorer
5. Verify JSON structure sesuai format di atas

### **Step 3: Backend Validation**

Setelah ESP32 publish, cek Railway backend logs:

**Expected logs:**
```
[INFO] MQTT message received from topic: enose/ESP32-001/measurement
[INFO] Parsed device_id: ESP32-001
[INFO] Sensor values: mq2=450, mq3=320, mq4=280, ...
[INFO] Data saved to database successfully
```

**Error logs (jika ada masalah):**
```
[ERROR] Failed to parse JSON: missing field `device_id`
[ERROR] Invalid timestamp format: "23/09/2026 10:30:00"
[ERROR] Sensor value out of range: mq2=9999
```

### **Step 4: Dashboard Check**

1. Buka dashboard: `http://enose-cloud-backend-production.up.railway.app/dashboard`
2. Data harus muncul dalam **Latest Activity** table
3. Chart harus update realtime setiap ESP32 publish
4. Device ID harus sesuai dengan yang di-set di ESP32

---

## ⚠️ COMMON MISTAKES (JANGAN SAMPAI SALAH!)

### ❌ **Mistake #1: Broker Berbeda**
```rust
// ESP32
MQTT_BROKER = "test.mosquitto.org"

// Railway Backend
MQTT_BROKER = "broker.emqx.io"

// Result: Data TIDAK MASUK (broker berbeda!)
```

### ❌ **Mistake #2: Topic Format Salah**
```rust
// ESP32 publish ke:
Topic: "sensor/ESP32-001/data"

// Backend subscribe ke:
Topic: "enose/+/measurement"

// Result: Data TIDAK MASUK (topic tidak match!)
```

### ❌ **Mistake #3: JSON Field Name Salah**
```json
// ESP32 kirim:
{
  "device": "ESP32-001",      // ❌ Harus "device_id"
  "sensor1": 450              // ❌ Harus "mq2"
}

// Backend expect:
{
  "device_id": "ESP32-001",   // ✅
  "mq2": 450                  // ✅
}

// Result: Data masuk tapi GAGAL PARSE!
```

### ❌ **Mistake #4: Timestamp Format Salah**
```json
{
  "timestamp": "23/09/2026 10:30:00"  // ❌ Format Indonesia
}

// Harus:
{
  "timestamp": "2026-09-23T10:30:00Z" // ✅ ISO 8601
}
```

### ❌ **Mistake #5: Device ID Tidak Konsisten**
```rust
// Topic
Topic: "enose/ESP32-001/measurement"

// Payload
{
  "device_id": "ESP32-002"  // ❌ Beda dengan topic!
}

// Harus sama:
Topic: "enose/ESP32-001/measurement"
Payload: { "device_id": "ESP32-001" } // ✅
```

### ❌ **Mistake #6: Sensor Values String**
```json
{
  "mq2": "450",   // ❌ String (ada quotes)
  "mq3": "320"
}

// Harus:
{
  "mq2": 450,     // ✅ Number (tanpa quotes)
  "mq3": 320
}
```

### ❌ **Mistake #7: QoS 0 dengan Network Jelek**
```rust
// QoS 0 = at most once (bisa hilang)
mqtt_client.publish(topic, QoS::AtMostOnce, ...);

// Recommended: QoS 1 = at least once (reliable)
mqtt_client.publish(topic, QoS::AtLeastOnce, ...); // ✅
```

---

## 📊 EXPECTED DATABASE SCHEMA

Setelah data masuk, backend akan simpan ke PostgreSQL dengan struktur:

```sql
Table: measurements

Column          | Type                  | Description
----------------|-----------------------|----------------------------------
id              | UUID                  | Primary key (auto-generated)
device_id       | VARCHAR(100)          | ESP32 device identifier
timestamp       | TIMESTAMP WITH TZ     | Waktu measurement (UTC)
mq2             | INTEGER               | MQ-2 sensor value
mq3             | INTEGER               | MQ-3 sensor value
mq4             | INTEGER               | MQ-4 sensor value
mq5             | INTEGER               | MQ-5 sensor value
mq6             | INTEGER               | MQ-6 sensor value
mq7             | INTEGER               | MQ-7 sensor value
mq8             | INTEGER               | MQ-8 sensor value
mq135           | INTEGER               | MQ-135 sensor value
prediction      | VARCHAR(100)          | ML prediction result (optional)
confidence      | FLOAT                 | ML confidence score (optional)
created_at      | TIMESTAMP             | Record creation time (auto)
```

---

## 🔐 SECURITY NOTES

1. **Public Broker:** `broker.emqx.io` adalah public broker tanpa autentikasi
   - ⚠️ Jangan kirim data sensitif/pribadi
   - ✅ Aman untuk sensor data (non-critical)

2. **TLS/SSL:** Saat ini tidak pakai TLS (port 1883)
   - Jika mau secure, gunakan port 8883 dengan certificate

3. **API Keys:** Tidak ada authentication di MQTT layer
   - Backend Railway tidak expose credentials

---

## 📞 TROUBLESHOOTING

### **Problem: ESP32 tidak bisa connect ke MQTT broker**

**Checklist:**
1. ✅ WiFi connected? (cek serial log)
2. ✅ Broker address: `broker.emqx.io` (bukan mosquitto atau localhost)
3. ✅ Port: `1883` (bukan 8883)
4. ✅ Firewall/network allow outgoing port 1883?

### **Problem: Data tidak masuk ke database**

**Checklist:**
1. ✅ Topic format: `enose/{device_id}/measurement`
2. ✅ JSON valid? (test dengan JSON validator)
3. ✅ Field names sesuai: `device_id`, `mq2`-`mq8`, `mq135`, `timestamp`
4. ✅ Timestamp ISO 8601: `2026-09-23T10:30:00Z`
5. ✅ Sensor values = numbers (bukan string)
6. ✅ Cek Railway backend logs untuk error message

### **Problem: Confidence selalu 0.00%**

**Checklist:**
1. ✅ ML model Edge Impulse sudah di-deploy ke ESP32?
2. ✅ Field `prediction` dan `confidence` di-kirim dalam payload?
3. ✅ Confidence format: `0.95` (float 0.0-1.0), bukan `95` (integer)

---

## 📚 REFERENCES

- **MQTT Protocol:** https://mqtt.org/
- **ISO 8601 Timestamp:** https://en.wikipedia.org/wiki/ISO_8601
- **JSON Format:** https://www.json.org/
- **Edge Impulse:** https://edgeimpulse.com/
- **Backend Repository:** (contact Bagian C for access)

---

## ✅ FINAL CHECKLIST

Sebelum deploy ESP32 ke production, pastikan semua ini sudah ✅:

```
☑️ WiFi credentials configured
☑️ MQTT broker: broker.emqx.io
☑️ MQTT port: 1883
☑️ MQTT QoS: 1
☑️ Topic format: enose/{device_id}/measurement
☑️ device_id konsisten (topic = payload)
☑️ JSON field names sesuai spec
☑️ Timestamp ISO 8601 (UTC/WIB)
☑️ Sensor values = number (bukan string)
☑️ ML model deployed & confidence 0.0-1.0
☑️ Publish rate max 1 Hz (1 msg/second)
☑️ Error handling (WiFi & MQTT reconnect)
☑️ Serial logging untuk debugging
☑️ Tested dengan MQTT Explorer
☑️ Verified data masuk di Railway dashboard
```

---

## 🚀 DEPLOYMENT

Setelah semua checklist ✅, flash firmware ke ESP32:

```bash
# Build firmware
cargo build --release --target xtensa-esp32-espidf

# Flash to ESP32
espflash flash target/xtensa-esp32-espidf/release/your-firmware

# Monitor serial output
espflash monitor
```

**Expected output:**
```
[INFO] WiFi connected!
[INFO] MQTT connected to broker.emqx.io:1883
[INFO] [MQTT PUBLISHED] enose/ESP32-001/measurement
[INFO] [MQTT PUBLISHED] enose/ESP32-001/measurement
...
```

---

## 📧 CONTACT

Jika ada pertanyaan atau masalah:
- **Bagian C (Backend):** [Your contact info]
- **MQTT Broker Issues:** Check broker.emqx.io status
- **Edge Impulse ML:** https://docs.edgeimpulse.com/

---

**Last Updated:** 2026-09-23  
**Document Version:** 1.0  
**Target Firmware:** ESP32 (Rust no_std)  
**Backend Version:** Railway Cloud (Rust Axum + PostgreSQL)

---

**Good luck! 🚀☕**
