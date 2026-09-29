//! A game's own SGB border, loaded through its CHR_TRN and PCT_TRN packets.

use missingno_gb::cartridge::Cartridge;
use missingno_gb::sgb::{GAME_BOY_ORIGIN, SNES_HEIGHT, SNES_WIDTH};
use missingno_gb::system::ConsoleUi;
use missingno_gb::{Dmg, GameBoy};

fn run_frame(gb: &mut GameBoy) {
    while !gb.step().new_screen {}
}

/// Pokémon Red sends its border as it starts; give its path in `SGB_BORDER_ROM`.
#[test]
#[ignore = "needs a commercial SGB ROM in SGB_BORDER_ROM"]
fn a_games_border_frames_the_game_boy_and_survives_a_restore() {
    let path = std::env::var("SGB_BORDER_ROM").expect("SGB_BORDER_ROM names a ROM");
    let rom = std::fs::read(&path).unwrap();
    let boot = || GameBoy::new(Cartridge::new(rom.clone(), None, None).unwrap(), None);
    let mut original = boot();

    let opaque = |gb: &GameBoy, x: usize, y: usize| gb.sgb().unwrap().border().color_at(x, y);
    let mut frames = 0;
    while (0..SNES_WIDTH).all(|x| opaque(&original, x, 0).is_none()) {
        run_frame(&mut original);
        frames += 1;
        assert!(frames < 1200, "no border arrived");
    }
    let (left, top) = GAME_BOY_ORIGIN;
    for y in top..top + 144 {
        for x in left..left + 160 {
            assert_eq!(
                opaque(&original, x, y),
                None,
                "the border covers ({x}, {y})"
            );
        }
    }

    let record = <Dmg as ConsoleUi>::read_state(&original).unwrap();
    let memory = <Dmg as ConsoleUi>::capture_memory(&original)
        .into_iter()
        .map(|(name, data)| (name.to_owned(), data))
        .collect();
    let mut restored = boot();
    restored.restore_boundary(&record, memory, None).unwrap();
    for _ in 0..60 {
        run_frame(&mut original);
        run_frame(&mut restored);
    }
    let picture = |gb: &GameBoy| gb.sgb().unwrap().snes_picture();
    assert_eq!(picture(&restored), picture(&original));
    assert_eq!(picture(&original).len(), SNES_WIDTH * SNES_HEIGHT);
}
