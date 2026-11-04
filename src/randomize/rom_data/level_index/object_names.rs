//! Object ID names, from the `OBJ_*` constants in the Southbird disassembly
//! (`smb3.asm`, prefix dropped). `object_names_match_the_disassembly` checks
//! this table against the disassembly wherever the local clone exists.

/// Every named object ID, ascending.
const OBJECT_NAMES: &[(u8, &str)] = &[
    (0x06, "BOUNCEDOWNUP"),
    (0x07, "WARPHIDE"),
    (0x08, "PSWITCHDOOR"),
    (0x09, "AIRSHIPANCHOR"),
    (0x0B, "POWERUP_1UP"),
    (0x0C, "POWERUP_STARMAN"),
    (0x0D, "POWERUP_MUSHROOM"),
    (0x0E, "BOSS_KOOPALING"),
    (0x17, "SPINYCHEEP"),
    (0x18, "BOSS_BOWSER"),
    (0x19, "POWERUP_FIREFLOWER"),
    (0x1B, "BOUNCELEFTRIGHT"),
    (0x1E, "POWERUP_SUPERLEAF"),
    (0x1F, "GROWINGVINE"),
    (0x21, "POWERUP_MUSHCARD"),
    (0x22, "POWERUP_FIRECARD"),
    (0x23, "POWERUP_STARCARD"),
    (0x24, "CLOUDPLATFORM_FAST"),
    (0x25, "PIPEWAYCONTROLLER"),
    (0x26, "WOODENPLAT_RIDER"),
    (0x27, "OSCILLATING_H"),
    (0x28, "OSCILLATING_V"),
    (0x29, "SPIKE"),
    (0x2A, "PATOOIE"),
    (0x2B, "GOOMBAINSHOE"),
    (0x2C, "CLOUDPLATFORM"),
    (0x2D, "BIGBERTHA"),
    (0x2E, "INVISIBLELIFT"),
    (0x2F, "BOO"),
    (0x30, "HOTFOOT_SHY"),
    (0x31, "BOOSTRETCH"),
    (0x32, "BOOSTRETCH_FLIP"),
    (0x33, "NIPPER"),
    (0x34, "TOAD"),
    (0x35, "TOADHOUSEITEM"),
    (0x36, "WOODENPLATFORM"),
    (0x37, "OSCILLATING_HS"),
    (0x38, "OSCILLATING_VS"),
    (0x39, "NIPPERHOPPING"),
    (0x3A, "FALLINGPLATFORM"),
    (0x3B, "CHARGINGCHEEPCHEEP"),
    (0x3C, "WOODENPLATFORMFALL"),
    (0x3D, "NIPPERFIREBREATHER"),
    (0x3E, "WOODENPLATFORMFLOAT"),
    (0x3F, "DRYBONES"),
    (0x40, "BUSTERBEATLE"),
    (0x41, "ENDLEVELCARD"),
    (0x42, "CHEEPCHEEPPOOL2POOL"),
    (0x43, "CHEEPCHEEPPOOL2POOL2"),
    (0x44, "WOODENPLATUNSTABLE"),
    (0x45, "HOTFOOT"),
    (0x46, "PIRANHASPIKEBALL"),
    (0x47, "GIANTBLOCKCTL"),
    (0x48, "TINYCHEEPCHEEP"),
    (0x49, "FLOATINGBGCLOUD"),
    (0x4A, "BOOMBOOMQBALL"),
    (0x4B, "BOOMBOOMJUMP"),
    (0x4C, "BOOMBOOMFLY"),
    (0x4F, "CHAINCHOMPFREE"),
    (0x50, "BOBOMBEXPLODE"),
    (0x51, "ROTODISCDUAL"),
    (0x52, "TREASUREBOX"),
    (0x53, "PODOBOOCEILING"),
    (0x54, "DONUTLIFTSHAKEFALL"),
    (0x55, "BOBOMB"),
    (0x56, "PIRANHASIDEWAYSLEFT"),
    (0x57, "PIRANHASIDEWAYSRIGHT"),
    (0x58, "FIRECHOMP"),
    (0x59, "FIRESNAKE"),
    (0x5A, "ROTODISCCLOCKWISE"),
    (0x5B, "ROTODISCCCLOCKWISE"),
    (0x5C, "ICEBLOCK"),
    (0x5D, "TORNADO"),
    (0x5E, "ROTODISCDUALOPPOSE"),
    (0x5F, "ROTODISCDUALOPPOSE2"),
    (0x60, "ROTODISCDUALCCLOCK"),
    (0x61, "BLOOPERWITHKIDS"),
    (0x62, "BLOOPER"),
    (0x63, "BIGBERTHABIRTHER"),
    (0x64, "CHEEPCHEEPHOPPER"),
    (0x65, "WATERCURRENTUPWARD"),
    (0x66, "WATERCURRENTDOWNARD"),
    (0x67, "LAVALOTUS"),
    (0x68, "TWIRLINGBUZZY"),
    (0x69, "TWIRLINGSPINY"),
    (0x6A, "BLOOPERCHILDSHOOT"),
    (0x6B, "PILEDRIVER"),
    (0x6C, "GREENTROOPA"),
    (0x6D, "REDTROOPA"),
    (0x6E, "PARATROOPAGREENHOP"),
    (0x6F, "FLYINGREDPARATROOPA"),
    (0x70, "BUZZYBEATLE"),
    (0x71, "SPINY"),
    (0x72, "GOOMBA"),
    (0x73, "PARAGOOMBA"),
    (0x74, "PARAGOOMBAWITHMICROS"),
    (0x75, "BOSSATTACK"),
    (0x76, "JUMPINGCHEEPCHEEP"),
    (0x77, "GREENCHEEP"),
    (0x78, "BULLETBILL"),
    (0x79, "BULLETBILLHOMING"),
    (0x7A, "BIGGREENTROOPA"),
    (0x7B, "BIGREDTROOPA"),
    (0x7C, "BIGGOOMBA"),
    (0x7D, "BIGGREENPIRANHA"),
    (0x7E, "BIGGREENHOPPER"),
    (0x7F, "BIGREDPIRANHA"),
    (0x80, "FLYINGGREENPARATROOPA"),
    (0x81, "HAMMERBRO"),
    (0x82, "BOOMERANGBRO"),
    (0x83, "LAKITU"),
    (0x84, "SPINYEGG"),
    (0x85, "SPINYEGGDUD"),
    (0x86, "HEAVYBRO"),
    (0x87, "FIREBRO"),
    (0x88, "ORANGECHEEP"),
    (0x89, "CHAINCHOMP"),
    (0x8A, "THWOMP"),
    (0x8B, "THWOMPLEFTSLIDE"),
    (0x8C, "THWOMPRIGHTSLIDE"),
    (0x8D, "THWOMPUPDOWN"),
    (0x8E, "THWOMPDIAGONALUL"),
    (0x8F, "THWOMPDIAGONALDL"),
    (0x90, "TILTINGPLATFORM"),
    (0x91, "TWIRLINGPLATCWNS"),
    (0x92, "TWIRLINGPLATCW"),
    (0x93, "TWIRLINGPERIODIC"),
    (0x94, "BIGQBLOCK_3UP"),
    (0x95, "BIGQBLOCK_MUSHROOM"),
    (0x96, "BIGQBLOCK_FIREFLOWER"),
    (0x97, "BIGQBLOCK_SUPERLEAF"),
    (0x98, "BIGQBLOCK_TANOOKI"),
    (0x99, "BIGQBLOCK_FROG"),
    (0x9A, "BIGQBLOCK_HAMMER"),
    (0x9D, "FIREJET_UPWARD"),
    (0x9E, "PODOBOO"),
    (0x9F, "PARABEETLE"),
    (0xA0, "GREENPIRANHA"),
    (0xA1, "GREENPIRANHA_FLIPPED"),
    (0xA2, "REDPIRANHA"),
    (0xA3, "REDPIRANHA_FLIPPED"),
    (0xA4, "GREENPIRANHA_FIRE"),
    (0xA5, "GREENPIRANHA_FIREC"),
    (0xA6, "VENUSFIRETRAP"),
    (0xA7, "VENUSFIRETRAP_CEIL"),
    (0xA8, "ARROWONE"),
    (0xA9, "ARROWANY"),
    (0xAA, "AIRSHIPPROP"),
    (0xAC, "FIREJET_LEFT"),
    (0xAD, "ROCKYWRENCH"),
    (0xAE, "BOLTLIFT"),
    (0xAF, "ENEMYSUN"),
    (0xB0, "BIGCANNONBALL"),
    (0xB1, "FIREJET_RIGHT"),
    (0xB2, "FIREJET_UPSIDEDOWN"),
    (0xB4, "CHEEPCHEEPBEGIN"),
    (0xB5, "GREENCHEEPBEGIN"),
    (0xB6, "LAKITUFLEE"),
    (0xB7, "PARABEETLESBEGIN"),
    (0xB8, "CLOUDSINBGBEGIN"),
    (0xB9, "WOODPLATFORMBEGIN"),
    (0xBA, "TREASUREBOXAPPEAR"),
    (0xBB, "CANCELEVENT"),
    (0xBC, "CFIRE_BULLETBILL"),
    (0xBD, "CFIRE_MISSILEBILL"),
    (0xBE, "CFIRE_ROCKYWRENCH"),
    (0xBF, "CFIRE_4WAY"),
    (0xC0, "CFIRE_GOOMBAPIPE_L"),
    (0xC1, "CFIRE_GOOMBAPIPE_R"),
    (0xC2, "CFIRE_HLCANNON"),
    (0xC3, "CFIRE_HLBIGCANNON"),
    (0xC4, "CFIRE_ULCANNON"),
    (0xC5, "CFIRE_URCANNON"),
    (0xC6, "CFIRE_LLCANNON"),
    (0xC7, "CFIRE_LRCANNON"),
    (0xC8, "CFIRE_HLCANNON2"),
    (0xC9, "CFIRE_ULCANNON2"),
    (0xCA, "CFIRE_URCANNON2"),
    (0xCB, "CFIRE_LLCANNON2"),
    (0xCC, "CFIRE_HRCANNON"),
    (0xCD, "CFIRE_HRBIGCANNON"),
    (0xCE, "CFIRE_LBOBOMBS"),
    (0xCF, "CFIRE_RBOBOMBS"),
    (0xD0, "CFIRE_LASER"),
    (0xD1, "SPAWN3GREENTROOPAS"),
    (0xD2, "SPAWN3ORANGECHEEPS"),
    (0xD3, "AUTOSCROLL"),
    (0xD4, "BONUSCONTROLLER"),
    (0xD5, "TOADANDKING"),
    (0xD6, "TREASURESET"),
];

/// The disassembly's name for `id`, if it has one.
pub(crate) fn object_name(id: u8) -> Option<&'static str> {
    OBJECT_NAMES.binary_search_by_key(&id, |&(i, _)| i).ok().map(|i| OBJECT_NAMES[i].1)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DISASM: &str = "tools/southbird-smb3/smb3.asm";

    /// Parse a `$NN` or decimal literal.
    fn literal(s: &str) -> Option<u8> {
        match s.strip_prefix('$') {
            Some(hex) => u8::from_str_radix(hex, 16).ok(),
            None => s.parse().ok(),
        }
    }

    /// `(name, value-expression)` for every `PREFIX_X = expr` line.
    fn constants<'a>(src: &'a str, prefix: &str) -> Vec<(&'a str, &'a str)> {
        src.lines()
            .filter(|l| l.starts_with(prefix))
            .filter_map(|l| {
                let l = l.split(';').next()?;
                let (name, expr) = l.split_once('=')?;
                Some((name.trim(), expr.trim()))
            })
            .collect()
    }

    /// Every `OBJ_*` id and name in the disassembly. Plain literals, plus the
    /// cannon fires' `$BC + CFIRE_X - 1`.
    fn from_disassembly(src: &str) -> Vec<(u8, String)> {
        let cfire: std::collections::HashMap<&str, u8> = constants(src, "CFIRE_")
            .into_iter()
            .filter_map(|(n, e)| Some((n, literal(e)?)))
            .collect();
        let mut out: Vec<(u8, String)> = constants(src, "OBJ_")
            .into_iter()
            .map(|(name, expr)| {
                let id = literal(expr).unwrap_or_else(|| {
                    let parts: Vec<&str> = expr.split_whitespace().collect();
                    assert!(
                        matches!(parts[..], [_, "+", _, "-", "1"]),
                        "{name}: unexpected expression {expr}"
                    );
                    literal(parts[0]).unwrap() + cfire[parts[2]] - 1
                });
                (id, name.trim_start_matches("OBJ_").to_string())
            })
            .collect();
        out.sort();
        out
    }

    #[test]
    fn object_names_match_the_disassembly() {
        let Ok(src) = std::fs::read_to_string(DISASM) else { return };
        let want = from_disassembly(&src);
        let have: Vec<(u8, String)> =
            OBJECT_NAMES.iter().map(|&(i, n)| (i, n.to_string())).collect();
        assert_eq!(have, want);
    }

    /// Prints the table body. `cargo test --lib print_object_names -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn print_object_names() {
        let src = std::fs::read_to_string(DISASM).unwrap();
        for (id, name) in from_disassembly(&src) {
            println!("    (0x{id:02X}, \"{name}\"),");
        }
    }
}
