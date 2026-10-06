#![allow(dead_code)]

use embedded_storage::nor_flash::{NorFlash, ReadNorFlash};
use esp_storage::FlashStorage;
use esp_println::println;

pub const OTADATA_OFFSET: u32 = 0x0000_F000;
pub const OTA_0_OFFSET: u32   = 0x0002_0000;
pub const OTA_1_OFFSET: u32   = 0x0032_0000;
pub const SECTOR_SIZE: u32    = 4096;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct EspOtaSelectEntry {
    pub ota_seq: u32,
    pub seq_label: [u8; 20],
    pub ota_state: u32,
    pub crc: u32,
}

fn esp_crc32(data: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            if (crc & 1) != 0 {
                crc = (crc >> 1) ^ 0xEDB8_8320;
            } else {
                crc >>= 1;
            }
        }
    }
    !crc
}

pub struct OtaUpdater {
    flash: FlashStorage,
    pub target_offset: u32,
    pub current_write_offset: u32,
    pub next_seq: u32,
}

impl OtaUpdater {
    pub fn new() -> Self {
        let mut flash = FlashStorage::new();
        let mut otadata_buf = [0u8; 32];
        let _ = flash.read(OTADATA_OFFSET, &mut otadata_buf);

        let seq_0 = u32::from_le_bytes(otadata_buf[0..4].try_into().unwrap_or([0xFF; 4]));

        let (target_offset, next_seq) = if seq_0 == 1 {
            println!("OTA: Berjalan di OTA_1. Target update -> OTA_0");
            (OTA_0_OFFSET, 2)
        } else {
            println!("OTA: Berjalan di OTA_0. Target update -> OTA_1");
            (OTA_1_OFFSET, 1)
        };

        Self {
            flash,
            target_offset,
            current_write_offset: target_offset,
            next_seq,
        }
    }

    pub fn write_chunk(&mut self, data: &[u8]) -> Result<(), ()> {
        let mut remaining = data;
        while !remaining.is_empty() {
            if self.current_write_offset % SECTOR_SIZE == 0 {
                let _ = self.flash.erase(self.current_write_offset, self.current_write_offset + SECTOR_SIZE);
            }
            let bytes_in_sector = (SECTOR_SIZE - (self.current_write_offset % SECTOR_SIZE)) as usize;
            let to_write = core::cmp::min(remaining.len(), bytes_in_sector);
            self.flash.write(self.current_write_offset, &remaining[..to_write]).map_err(|_| ())?;
            self.current_write_offset += to_write as u32;
            remaining = &remaining[to_write..];
        }
        Ok(())
    }

    pub fn finalize_and_switch(&mut self) -> Result<(), ()> {
        println!("OTA: Memperbarui otadata ke Sequence: {}", self.next_seq);

        let mut entry = EspOtaSelectEntry {
            ota_seq: self.next_seq,
            seq_label: [0xFF; 20],
            ota_state: 0,
            crc: 0,
        };

        let entry_bytes: [u8; 28] = unsafe { core::mem::transmute_copy(&entry) };
        entry.crc = esp_crc32(&entry_bytes);

        let final_bytes: [u8; 32] = unsafe { core::mem::transmute(entry) };

        self.flash.erase(OTADATA_OFFSET, OTADATA_OFFSET + SECTOR_SIZE).map_err(|_| ())?;
        self.flash.write(OTADATA_OFFSET, &final_bytes).map_err(|_| ())?;

        println!("OTA: Sukses! Bootloader disetel ke partisi baru.");
        Ok(())
    }
}