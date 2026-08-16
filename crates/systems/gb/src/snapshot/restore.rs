//! Rebuilding the console in place from a boundary snapshot.

use missingno_core::state::StateRecord;
use missingno_core::state_file::StateFrame;
use missingno_core::system::StateError;

use super::{MbcSnapshot, Snapshot, clock_register_from_code, parse_record};
use crate::audio::Audio;
use crate::cartridge::mbc::Mbc;
use crate::interrupts::InterruptFlags;
use crate::{Console, Model, ScreenBuffer};

impl<M: Model> Console<M> {
    /// Restore this console in place from a validated record at an instruction
    /// boundary: the shared subsystems, the model's banked memory and register
    /// delta, and the displayed framebuffer. Errors (never panics) on a
    /// mid-instruction call or a record this model cannot faithfully restore.
    pub fn restore_boundary(
        &mut self,
        record: &StateRecord,
        memory: Vec<(String, Vec<u8>)>,
        frame: Option<&StateFrame>,
    ) -> Result<(), StateError> {
        // A save is taken between instructions: the CPU is either about to fetch
        // or halted (waiting on an interrupt). Both are clean boundaries; a
        // mid-instruction or speed-switch-stopped console is not restorable.
        if !self.cpu().is_fetch_phase() && !self.cpu().is_halted() {
            return Err(StateError::NotAtBoundary);
        }
        self.model.validate_boundary(record)?;
        let snapshot = parse_record(record, memory)?;
        self.restore_snapshot(&snapshot);
        self.model
            .restore_boundary_delta(&mut self.chassis, record, &snapshot.memory)?;
        // Seed the displayed screen from the saved framebuffer so the first
        // frame after a restore matches the save.
        if let Some(frame) = frame {
            self.chassis.screen.restore(&frame.data);
        }
        Ok(())
    }

    /// Rebuild the shared subsystems in place from a boundary snapshot, keeping
    /// the existing cartridge (the ROM) and re-seating every subsystem at an
    /// instruction boundary: the clock is placed at the single-speed `Rise`
    /// phase and the volatile bus/pixel state is defaulted. Model-specific state
    /// (CGB banks, palette RAM, speed) is reseated separately by
    /// [`Model::restore_boundary_delta`].
    pub fn restore_snapshot(&mut self, snap: &Snapshot) {
        use crate::memory::VramBus;
        use crate::ppu::memory::{Oam, Vram};
        use crate::ppu::model::PpuModel;

        let region = |name: &str| -> Option<&[u8]> {
            snap.memory
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, data)| data.as_slice())
        };

        let mut wave_ram = [0u8; 16];
        if let Some(data) = region("wave_ram") {
            let len = data.len().min(16);
            wave_ram[..len].copy_from_slice(&data[..len]);
        }

        // Work RAM lands where the model keeps it (DMG flat bus, CGB banks);
        // cartridge RAM lands in the existing external bus.
        if let Some(wram) = region("wram") {
            self.model
                .restore_work_ram(&mut self.chassis.external, wram);
        }
        if let Some(cart_ram) = region("cart_ram") {
            // A linear all-banks raw restore: independent of the mapper's enable
            // latch and bank window, so it neither overflows the 16-bit bus
            // address nor drops banks past the one currently mapped.
            self.chassis.external.cartridge.restore_ram(cart_ram);
        }
        restore_mbc(&snap.mbc, self.chassis.external.cartridge.mbc_mut());

        let oam = region("oam").map(Oam::from_bytes).unwrap_or_default();

        self.chassis.cpu = crate::cpu::Cpu::from_snapshot(&snap.cpu);
        self.chassis.ppu = crate::ppu::Ppu::from_snapshot(&snap.ppu, oam);
        self.chassis.audio = Audio::from_snapshot(&snap.apu, wave_ram);
        self.chassis.timers = crate::timers::Timers::from_snapshot(&snap.timer);
        self.chassis.dma = crate::dma::Dma::from_snapshot(&snap.dma);
        // The OAM-DMA source register (FF46) reads back independently of an
        // in-flight transfer, so restore it from the captured register value.
        self.chassis.dma.set_source_register(snap.ppu.dma);
        self.chassis.serial = crate::serial_transfer::Serial::from_snapshot(&snap.serial);
        self.chassis.interrupts = {
            let mut regs = crate::interrupts::Registers::new();
            regs.enabled = InterruptFlags::from_bits_retain(snap.cpu.ie);
            regs.requested = InterruptFlags::from_bits_retain(snap.cpu.if_);
            regs
        };
        let mut vram = <<M::Ppu as PpuModel>::Vram>::default();
        if let Some(bytes) = region("vram") {
            vram.restore_image(bytes);
        }
        self.chassis.vram_bus = VramBus { vram, latch: 0xFF };
        self.chassis.high_ram = region("hram")
            .map(crate::memory::HighRam::from_bytes)
            .unwrap_or_else(crate::memory::HighRam::new);

        self.chassis.screen = M::Screen::default();
        self.chassis.bus_trace = crate::cpu_bus::BusTrace::new();
        self.chassis.clock = crate::MasterClock::new(crate::CpuDivider::One);
        self.chassis.cpu_bus = crate::cpu_bus::CpuBus::new();
        self.chassis.dma_conflict = crate::DmaConflictLatch::default();
        self.chassis.joypad = crate::joypad::Joypad::new();
    }
}

fn restore_mbc(snap: &MbcSnapshot, mbc: &mut Mbc) {
    use crate::cartridge::mbc::mbc3::{ClockRegisters, Mapped};
    match mbc {
        Mbc::NoMbc(_) => {}
        Mbc::Mbc1(m) => {
            m.bank = snap.rom_bank as u8;
            m.ram_bank = snap.ram_bank;
            m.ram_enabled = snap.ram_enabled;
            m.mode1 = snap.mode != 0;
        }
        Mbc::Mbc2(m) => {
            m.bank = snap.rom_bank as u8;
            m.ram_enabled = snap.ram_enabled;
        }
        Mbc::Mbc3(m) => {
            m.bank = snap.rom_bank as u8;
            m.ram_and_clock_enabled = snap.ram_enabled;
            // Reseat the $A000 window's RAM-bank-vs-clock selection, then the
            // clock register file itself.
            m.mapped = match snap.clock_register {
                Some(code) => Mapped::Clock(clock_register_from_code(code)),
                None => Mapped::Ram(snap.ram_bank),
            };
            if let (Some(clock), Some(rtc)) = (m.clock.as_mut(), snap.rtc.as_ref()) {
                clock.registers = ClockRegisters {
                    seconds: rtc.seconds,
                    minutes: rtc.minutes,
                    hours: rtc.hours,
                    days_lower: rtc.day_lower,
                    days_upper: rtc.day_upper,
                };
                clock.latched = ClockRegisters {
                    seconds: rtc.latched_seconds,
                    minutes: rtc.latched_minutes,
                    hours: rtc.latched_hours,
                    days_lower: rtc.latched_day_lower,
                    days_upper: rtc.latched_day_upper,
                };
                clock.latch_ready = rtc.latch_ready;
            }
        }
        Mbc::Mbc5(m) => {
            m.rom_bank = snap.rom_bank;
            m.ram_bank = snap.ram_bank;
            m.ram_enabled = snap.ram_enabled;
            m.rumble = snap.mode != 0;
        }
        Mbc::Mbc6(m) => {
            m.rom_bank_a = snap.rom_bank as u8;
            m.ram_bank_a = snap.ram_bank;
            m.ram_enabled = snap.ram_enabled;
            if let Some(x) = &snap.mbc6 {
                m.rom_bank_b = x.rom_bank_b;
                m.ram_bank_b = x.ram_bank_b;
                m.rom_bank_a_flash = x.rom_a_flash;
                m.rom_bank_b_flash = x.rom_b_flash;
                m.flash_enabled = x.flash_enabled;
            }
        }
        Mbc::Mbc7(m) => {
            m.rom_bank = snap.rom_bank as u8;
            match &snap.mbc7 {
                Some(x) => {
                    m.ram_enabled_1 = x.ram_enabled_1;
                    m.ram_enabled_2 = x.ram_enabled_2;
                    m.accel_x = x.accel_x;
                    m.accel_y = x.accel_y;
                    m.eeprom.write_enabled = x.write_enabled;
                }
                None => {
                    m.ram_enabled_1 = snap.ram_enabled;
                    m.ram_enabled_2 = snap.ram_enabled;
                }
            }
        }
        Mbc::Huc1(m) => {
            m.rom_bank = snap.rom_bank as u8;
            m.ram_bank = snap.ram_bank;
            m.ir_mode = snap.mode != 0;
        }
        Mbc::Huc3(m) => {
            m.rom_bank = snap.rom_bank as u8;
            m.ram_bank = snap.ram_bank;
        }
        Mbc::DbzTrans(m) => m.restore(snap.rom_bank, snap.ram_bank, snap.ram_enabled),
    }
}

#[cfg(test)]
mod tests {
    use missingno_core::state::StateRecord;

    use crate::GameBoy;
    use crate::cartridge::Cartridge;
    use crate::cartridge::mbc::Mbc;
    use crate::cartridge::mbc::mbc3::Mapped;
    use crate::snapshot::{capture_memory, read_shared_record};

    /// A synthetic MBC3 ROM: header cartridge-type and RAM-size codes, a valid
    /// entry, everything else NOP.
    fn mbc3_rom(cart_type: u8, ram_code: u8) -> Vec<u8> {
        let mut rom = vec![0u8; 0x8000];
        rom[0x100] = 0x00;
        rom[0x101..0x104].copy_from_slice(&[0xc3, 0x50, 0x01]); // JP $0150
        rom[0x147] = cart_type;
        rom[0x149] = ram_code;
        rom
    }

    /// Capture a console's record and its owned memory spans, as the save path
    /// does.
    fn capture(console: &GameBoy) -> (StateRecord, Vec<(String, Vec<u8>)>) {
        let record = read_shared_record(console);
        let memory = capture_memory(console)
            .into_iter()
            .map(|(name, data)| (name.to_owned(), data))
            .collect();
        (record, memory)
    }

    // ── Finding 1: linear cart-RAM restore ───────────────────────────

    #[test]
    fn mbc3_32k_ram_round_trips_across_all_banks() {
        // 32 KiB (four 8 KiB banks), each stamped distinctly. The old restore
        // replayed this through the $A000 window and panicked past bank 2 (the
        // 16-bit bus address overflowed) while dropping every unmapped bank.
        let mut save = vec![0u8; 4 * 8 * 1024];
        for (bank, chunk) in save.chunks_mut(8 * 1024).enumerate() {
            chunk.fill(0x10 + bank as u8);
            chunk[0] = 0xA0 + bank as u8;
            *chunk.last_mut().unwrap() = 0xB0 + bank as u8;
        }
        let rom = mbc3_rom(0x13, 3); // MBC3+RAM+BATTERY, 32 KiB
        let source = GameBoy::new(Cartridge::new(rom.clone(), Some(save.clone())), None);
        let (record, memory) = capture(&source);

        let mut target = GameBoy::new(Cartridge::new(rom, None), None);
        target
            .restore_boundary(&record, memory, None)
            .expect("restore succeeds without panicking");

        for (offset, &byte) in save.iter().enumerate() {
            assert_eq!(
                target.cartridge().peek_ram(offset),
                byte,
                "cart RAM byte {offset:#x} (bank {}) diverged",
                offset / (8 * 1024)
            );
        }
    }

    #[test]
    fn cart_ram_restores_even_when_disabled_at_load() {
        // The source's RAM was never enabled (loaded straight from a battery
        // save), and the fresh target has RAM disabled too — the raw restore is
        // enable-independent, so the bytes still land.
        let mut save = vec![0u8; 8 * 1024];
        save[0x100] = 0x5A;
        let rom = mbc3_rom(0x13, 2); // MBC3+RAM+BATTERY, 8 KiB
        let source = GameBoy::new(Cartridge::new(rom.clone(), Some(save)), None);
        let (record, memory) = capture(&source);

        let mut target = GameBoy::new(Cartridge::new(rom, None), None);
        // The fresh target has cartridge RAM disabled.
        match target.cartridge().mbc() {
            Mbc::Mbc3(m) => assert!(!m.ram_and_clock_enabled),
            _ => panic!("expected MBC3"),
        }
        target
            .restore_boundary(&record, memory, None)
            .expect("restore");
        assert_eq!(target.cartridge().peek_ram(0x100), 0x5A);
    }

    // ── Finding 4: MBC3 mapped selection + RTC ───────────────────────

    #[test]
    fn mbc3_restore_lands_on_saved_ram_bank_over_a_live_clock() {
        let rom = mbc3_rom(0x10, 3); // MBC3+TIMER+RAM+BATTERY (carries a clock)

        // Source: RAM bank 2 mapped and enabled, a marker written to it.
        let mut src_cart = Cartridge::new(rom.clone(), None);
        src_cart.write(0x0000, 0x0A); // enable RAM + clock
        src_cart.write(0x4000, 0x02); // map RAM bank 2
        src_cart.write(0xA000, 0x77); // marker in bank 2
        let source = GameBoy::new(src_cart, None);
        let (record, memory) = capture(&source);

        // Target: currently on the clock register, not RAM.
        let mut tgt_cart = Cartridge::new(rom, None);
        tgt_cart.write(0x0000, 0x0A);
        tgt_cart.write(0x4000, 0x08); // map the seconds clock register
        let mut target = GameBoy::new(tgt_cart, None);
        target
            .restore_boundary(&record, memory, None)
            .expect("restore");

        match target.cartridge().mbc() {
            Mbc::Mbc3(m) => assert!(
                matches!(m.mapped, Mapped::Ram(2)),
                "restore should reseat RAM bank 2 over the live clock mapping"
            ),
            _ => panic!("expected MBC3"),
        }
        assert_eq!(
            target.cartridge().read(0xA000),
            0x77,
            "bank-2 marker reads back"
        );
    }

    #[test]
    fn mbc3_rtc_registers_round_trip() {
        let rom = mbc3_rom(0x10, 2); // MBC3+TIMER+RAM+BATTERY
        let mut src_cart = Cartridge::new(rom.clone(), None);
        src_cart.write(0x0000, 0x0A);
        src_cart.write(0x4000, 0x08); // map seconds
        src_cart.write(0xA000, 41); // seconds := 41
        src_cart.write(0x4000, 0x0A); // map hours
        src_cart.write(0xA000, 7); // hours := 7
        let source = GameBoy::new(src_cart, None);
        let (record, memory) = capture(&source);

        let mut target = GameBoy::new(Cartridge::new(rom, None), None);
        target
            .restore_boundary(&record, memory, None)
            .expect("restore");
        match target.cartridge().mbc() {
            Mbc::Mbc3(m) => {
                let clock = m.clock.as_ref().expect("clock present");
                assert_eq!(clock.registers.seconds, 41);
                assert_eq!(clock.registers.hours, 7);
            }
            _ => panic!("expected MBC3"),
        }
    }
}
