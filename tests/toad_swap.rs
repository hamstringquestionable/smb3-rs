use std::fs;
use std::path::Path;

const TOAD_IPS_PATH: &str = "web/visual-patches/super-toad-josuecr4ft.ips";
const USA_ROM_PATH: &str = "Super Mario Bros. 3 (USA) (Rev 1).nes";

fn read_optional(path: &str) -> Option<Vec<u8>> {
    if Path::new(path).exists() { Some(fs::read(path).unwrap()) } else { None }
}

#[test]
fn toad_ips_applies_to_usa_rev1_and_validates() {
    let Some(usa) = read_optional(USA_ROM_PATH) else {
        eprintln!("skipping: USA Rev 1 base ROM not present");
        return;
    };
    let ips = fs::read(TOAD_IPS_PATH).expect("bundled Toad IPS must exist");

    let patched = smb3_rs::apply_ips_patch(&usa, &ips).expect("apply_ips_patch must succeed");
    assert_eq!(patched.len(), usa.len(), "patched ROM size must equal base");

    // Patched ROM has a different payload CRC than vanilla Rev 1, so use lax
    // mode (we're verifying iNES layout, not revision).
    let rom = smb3_rs::rom::Rom::from_bytes_lax(&patched, true)
        .expect("patched ROM must validate");
    assert_eq!(rom.header.prg_pages, 16);
    assert_eq!(rom.header.chr_pages, 16);
    assert_eq!(rom.header.mapper, 4);

    // Spot-check a few palette bytes that the swap is documented to set
    // (Blue Toad: 0x22 in player palette table; red highlight at PRG027).
    assert_eq!(patched[16 + 0x326AE], 0x22, "Blue palette byte 1");
    assert_eq!(patched[16 + 0x33178], 0x22, "Blue palette byte 10");
    assert_eq!(patched[16 + 0x37838], 0x16, "Red highlight in PRG027");
}

#[test]
fn generate_patched_rom_layers_visual_patch_then_randomization() {
    // Verifies the new visual_patch parameter wiring: pass the original ROM
    // and the visual IPS to generate_patched_rom and expect both effects in
    // the output (visual palette bytes survive, randomization produces a
    // ROM that differs from vanilla in other places too).
    let Some(usa) = read_optional(USA_ROM_PATH) else {
        eprintln!("skipping: USA Rev 1 base ROM not present");
        return;
    };
    let ips = fs::read(TOAD_IPS_PATH).expect("bundled Toad IPS must exist");

    let options = smb3_rs::Options::default();
    let patched = smb3_rs::generate_patched_rom(&usa, 12345, &options, Some(&ips))
        .expect("generate_patched_rom must succeed with visual patch");

    assert_eq!(patched.len(), usa.len(), "output size must match input");

    // Visual-patch bytes must still be present in the final ROM.
    assert_eq!(patched[16 + 0x326AE], 0x22, "Blue palette byte 1 (visual)");
    assert_eq!(patched[16 + 0x33178], 0x22, "Blue palette byte 10 (visual)");
    assert_eq!(patched[16 + 0x37838], 0x16, "Red highlight in PRG027 (visual)");

    // And randomization must have actually run (output differs from a
    // visual-only patch). Compare against the visual-only baseline.
    let visual_only = smb3_rs::apply_ips_patch(&usa, &ips).unwrap();
    assert_ne!(
        patched, visual_only,
        "randomization should produce changes on top of the visual patch"
    );
}
