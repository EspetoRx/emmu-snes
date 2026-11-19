use std::borrow::Cow;

use meru_interface::{EmulatorCore, InputData};
use super_sabicom::Snes;

const BUTTONS: [&str; 12] = [
    "A", "B", "X", "Y", "L", "R", "Start", "Select", "Up", "Down", "Left", "Right",
];
const FRAME_WIDTH: usize = 256;
const FRAME_HEIGHT: usize = 224;

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
extern "C" {
    #[wasm_bindgen::prelude::wasm_bindgen(js_namespace = console)]
    fn log(s: &str);
}

fn log_snes(msg: &str) {
    eprintln!("{msg}");
    #[cfg(target_arch = "wasm32")]
    log(msg);
}

fn dump_header_window(rom: &[u8], start: usize, len: usize) -> String {
    if start >= rom.len() {
        return format!("0x{start:04X}: <out of bounds, len={}>", rom.len());
    }
    let end = (start + len).min(rom.len());
    let hex: Vec<String> = rom[start..end].iter().map(|b| format!("{b:02X}")).collect();
    format!("0x{start:04X}..0x{end:04X}: [{}]", hex.join(" "))
}

fn log_rom_diagnostics(stage: &str, rom: &[u8]) {
    let copier_mod = rom.len() & 0x3FF;
    let copier_len = if copier_mod == 0x200 { 0x200 } else { 0 };
    let logical_len = rom.len().saturating_sub(copier_len);
    let next_pow2 = logical_len.next_power_of_two();
    let is_pow2 = logical_len > 0 && logical_len == next_pow2;

    log_snes(&format!(
        "[SnesCore:{stage}] len={} (0x{:X}), len&0x3FF=0x{:03X}, copier_len=0x{:X}, logical_len={} (0x{:X}), is_pow2={}, next_pow2={}",
        rom.len(),
        rom.len(),
        copier_mod,
        copier_len,
        logical_len,
        logical_len,
        is_pow2,
        next_pow2,
    ));

    for base in [0x7F00_usize, 0x7FC0_usize, 0xFF00_usize, 0xFFC0_usize] {
        let raw_off = base;
        let with_copier_off = base + 0x200;
        log_snes(&format!(
            "[SnesCore:{stage}] {} | +0x200 {}",
            dump_header_window(rom, raw_off, 32),
            dump_header_window(rom, with_copier_off, 32),
        ));
    }
}

fn pad_non_pow2_rom(rom: &[u8]) -> Cow<'_, [u8]> {
    let copier_len = if (rom.len() & 0x3FF) == 0x200 {
        0x200
    } else {
        0
    };
    let logical_len = rom.len().saturating_sub(copier_len);
    let next_pow2 = logical_len.next_power_of_two();
    if logical_len == 0 || logical_len == next_pow2 {
        return Cow::Borrowed(rom);
    }

    let target_len = copier_len + next_pow2;
    let mut padded = Vec::with_capacity(target_len);
    padded.extend_from_slice(rom);
    padded.resize(target_len, 0x00);
    log_snes(&format!(
        "[SnesCore:pad_non_pow2_rom] padding aplicado: {} -> {} bytes (logical {} -> {})",
        rom.len(),
        padded.len(),
        logical_len,
        next_pow2
    ));
    Cow::Owned(padded)
}

fn repair_overlong_lorom_title(rom: &[u8]) -> Cow<'_, [u8]> {
    let copier_header_len = match rom.len() & 0x3FF {
        0 => 0,
        0x200 => 0x200,
        _ => return Cow::Borrowed(rom),
    };
    let header = copier_header_len + 0x7F00;
    let map_mode_index = header + 0xD5;
    if header + 0xE0 > rom.len() {
        return Cow::Borrowed(rom);
    }

    let map_mode = rom[map_mode_index];
    if map_mode & 0xE0 == 0x20 && matches!(map_mode & 0x0F, 0 | 1 | 2 | 3 | 5 | 0x0A) {
        return Cow::Borrowed(rom);
    }
    if !map_mode.is_ascii_alphabetic()
        || !rom[header + 0xC0..=map_mode_index]
            .iter()
            .all(|byte| (0x20..=0x7E).contains(byte))
    {
        return Cow::Borrowed(rom);
    }

    let chipset = rom[header + 0xD6];
    let rom_size_code = rom[header + 0xD7];
    let sram_size_code = rom[header + 0xD8];
    let chipset_is_valid = chipset <= 2 || matches!(chipset & 0x0F, 3 | 4 | 5 | 6 | 9);
    if !chipset_is_valid || rom_size_code > 0x0D || sram_size_code > 9 {
        return Cow::Borrowed(rom);
    }

    let logical_rom_len = rom.len() - copier_header_len;
    let declared_rom_len = 1024usize << rom_size_code;
    if declared_rom_len != logical_rom_len.next_power_of_two() {
        return Cow::Borrowed(rom);
    }

    let checksum_complement = u16::from_le_bytes([rom[header + 0xDC], rom[header + 0xDD]]);
    let checksum = u16::from_le_bytes([rom[header + 0xDE], rom[header + 0xDF]]);
    if checksum_complement != !checksum {
        return Cow::Borrowed(rom);
    }

    let mut repaired = rom.to_vec();
    repaired[map_mode_index] = 0x20;
    log_snes(&format!(
        "[SnesCore:repair_overlong_lorom_title] reparado byte 0x{map_mode_index:X}: 0x{map_mode:02X} -> 0x20"
    ));
    Cow::Owned(repaired)
}

fn repair_bsx_header(rom: &[u8]) -> Cow<'_, [u8]> {
    let copier_header_len = match rom.len() & 0x3FF {
        0 => 0,
        0x200 => 0x200,
        _ => return Cow::Borrowed(rom),
    };
    if copier_header_len + 0x8000 > rom.len() {
        return Cow::Borrowed(rom);
    }

    let header_pos = copier_header_len + 0x7F00;
    let checksum_complement = u16::from_le_bytes([rom[header_pos + 0xDC], rom[header_pos + 0xDD]]);
    let checksum = u16::from_le_bytes([rom[header_pos + 0xDE], rom[header_pos + 0xDF]]);
    if checksum_complement != !checksum {
        return Cow::Borrowed(rom);
    }

    if rom[header_pos + 0xDA] != 0x33 {
        return Cow::Borrowed(rom);
    }

    let map_mode_byte = rom[header_pos + 0xD5];
    let is_standard_map_mode =
        map_mode_byte & 0xE0 == 0x20 && matches!(map_mode_byte & 0x0F, 0 | 1 | 2 | 3 | 5 | 0x0A);
    let rom_size_code = rom[header_pos + 0xD7];
    let has_bsx_markers = rom[header_pos + 0xB0..header_pos + 0xB2] == *b"01"
        || &rom[header_pos + 0xC0..header_pos + 0xC2] == b"BS";

    if is_standard_map_mode && rom_size_code <= 0x0D {
        return Cow::Borrowed(rom);
    }

    if !has_bsx_markers && is_standard_map_mode {
        return Cow::Borrowed(rom);
    }

    let logical_rom_len = rom.len() - copier_header_len;
    let computed_size_code = (logical_rom_len.next_power_of_two() / 1024)
        .trailing_zeros()
        .min(0x0D) as u8;

    let mut repaired = rom.to_vec();

    for i in header_pos + 0xD0..header_pos + 0xD5 {
        repaired[i] = 0x20;
    }

    repaired[header_pos + 0xD5] = 0x20;
    repaired[header_pos + 0xD6] = 0x00;
    repaired[header_pos + 0xD7] = computed_size_code;
    repaired[header_pos + 0xD8] = 0x00;
    repaired[header_pos + 0xD9] = 0x00;
    repaired[header_pos + 0xDA] = 0x33;
    repaired[header_pos + 0xDB] = 0x00;

    let mut actual_sum = 0_u16;
    for i in copier_header_len..repaired.len() {
        let byte = if (header_pos + 0xDC..header_pos + 0xDE).contains(&i) {
            0xFF
        } else if (header_pos + 0xDE..header_pos + 0xE0).contains(&i) {
            0x00
        } else {
            repaired[i]
        };
        actual_sum = actual_sum.wrapping_add(byte as u16);
    }
    let complement = !actual_sum;
    repaired[header_pos + 0xDC] = complement as u8;
    repaired[header_pos + 0xDD] = (complement >> 8) as u8;
    repaired[header_pos + 0xDE] = actual_sum as u8;
    repaired[header_pos + 0xDF] = (actual_sum >> 8) as u8;

    log_snes(&format!(
        "[SnesCore:repair_bsx_header] reparado header em 0x{header_pos:X} (size_code=0x{computed_size_code:02X}, checksum=0x{actual_sum:04X})"
    ));
    Cow::Owned(repaired)
}

fn button_index(button: &str) -> Result<usize, String> {
    BUTTONS
        .iter()
        .position(|candidate| *candidate == button)
        .ok_or_else(|| format!("Botão SNES desconhecido: {button}"))
}

pub struct SnesCore {
    snes: Snes,
    p1_buttons: [bool; BUTTONS.len()],
    p2_buttons: [bool; BUTTONS.len()],
}

impl SnesCore {
    pub fn new(rom: &[u8]) -> Result<Self, super_sabicom::Error> {
        log_rom_diagnostics("input", rom);
        let rom = pad_non_pow2_rom(rom);
        let repaired_pad = matches!(&rom, Cow::Owned(_));
        let rom = repair_overlong_lorom_title(rom.as_ref());
        let repaired_lorom = matches!(&rom, Cow::Owned(_));
        let rom = repair_bsx_header(rom.as_ref());
        let repaired_bsx = matches!(&rom, Cow::Owned(_));

        if repaired_pad || repaired_lorom || repaired_bsx {
            log_rom_diagnostics("after_repairs", rom.as_ref());
        }

        log_snes("[SnesCore:new] chamando Snes::try_from_file...");
        let snes = match Snes::try_from_file(rom.as_ref(), None, &Default::default()) {
            Ok(instance) => {
                log_snes("[SnesCore:new] Snes::try_from_file concluiu com sucesso.");
                instance
            }
            Err(err) => {
                log_snes(&format!(
                    "[SnesCore:new] Snes::try_from_file retornou Err: {err}"
                ));
                return Err(err);
            }
        };

        Ok(Self {
            snes,
            p1_buttons: [false; BUTTONS.len()],
            p2_buttons: [false; BUTTONS.len()],
        })
    }

    pub fn game_title(&self) -> String {
        self.snes
            .game_info()
            .into_iter()
            .find_map(|(key, value)| (key == "Title").then_some(value))
            .unwrap_or_else(|| "SNES".to_string())
    }

    pub fn set_button(&mut self, button: &str, pressed: bool) -> Result<(), String> {
        self.set_button_for_player(button, pressed, 1)
    }

    pub fn set_button_for_player(
        &mut self,
        button: &str,
        pressed: bool,
        player: u8,
    ) -> Result<(), String> {
        let index = button_index(button)?;
        match player {
            1 => self.p1_buttons[index] = pressed,
            2 => self.p2_buttons[index] = pressed,
            _ => return Err(format!("Jogador SNES inválido: {player}")),
        }
        Ok(())
    }

    fn build_controller_input(buttons: &[bool; BUTTONS.len()]) -> Vec<(String, bool)> {
        BUTTONS
            .iter()
            .zip(*buttons)
            .map(|(name, pressed)| ((*name).to_string(), pressed))
            .collect()
    }

    fn apply_input(&mut self) {
        let controller1 = Self::build_controller_input(&self.p1_buttons);
        let controller2 = Self::build_controller_input(&self.p2_buttons);
        self.snes.set_input(&InputData {
            controllers: vec![controller1, controller2, Vec::new(), Vec::new()],
        });
    }

    pub fn run_frame(&mut self) -> Result<Vec<u8>, String> {
        self.apply_input();
        self.snes.exec_frame(true);

        let source = self.snes.frame_buffer();
        if source.width < FRAME_WIDTH * 2 || source.height < FRAME_HEIGHT * 2 {
            return Err(format!(
                "O núcleo retornou um framebuffer inválido: {}x{}.",
                source.width, source.height
            ));
        }
        let mut frame = Vec::with_capacity(FRAME_WIDTH * FRAME_HEIGHT * 4);
        for y in 0..FRAME_HEIGHT {
            let source_row = y * 2 * source.width;
            for x in 0..FRAME_WIDTH {
                let pixel = &source.buffer[source_row + x * 2];
                frame.extend_from_slice(&[pixel.r, pixel.g, pixel.b, 255]);
            }
        }
        Ok(frame)
    }

    pub fn run_frame_without_rendering(&mut self) {
        self.apply_input();
        self.snes.exec_frame(false);
    }

    pub fn audio_samples(&self) -> Vec<i16> {
        self.snes
            .audio_buffer()
            .samples
            .iter()
            .flat_map(|sample| [sample.left, sample.right])
            .collect()
    }

    pub fn audio_sample_rate(&self) -> u32 {
        self.snes.audio_buffer().sample_rate
    }

    pub fn save_state(&self) -> Vec<u8> {
        self.snes.save_state()
    }

    pub fn load_state(&mut self, state: &[u8]) -> Result<(), super_sabicom::Error> {
        self.snes.load_state(state)
    }
}

#[cfg(target_arch = "wasm32")]
mod wasm {
    use wasm_bindgen::prelude::*;

    use super::SnesCore;

    #[wasm_bindgen]
    pub struct SnesEmulator {
        core: SnesCore,
    }

    #[wasm_bindgen]
    impl SnesEmulator {
        #[wasm_bindgen(constructor)]
        pub fn new(rom: &[u8]) -> Result<SnesEmulator, JsValue> {
            super::log_snes(&format!(
                "[SnesEmulator::new] chamado com rom.len()={}",
                rom.len()
            ));
            let core = SnesCore::new(rom).map_err(|error| {
                let msg = error.to_string();
                super::log_snes(&format!(
                    "[SnesEmulator::new] erro mapeado para JsValue: {msg}"
                ));
                JsValue::from_str(&msg)
            })?;
            Ok(Self { core })
        }

        #[wasm_bindgen(js_name = gameTitle)]
        pub fn game_title(&self) -> String {
            self.core.game_title()
        }

        #[wasm_bindgen(js_name = setButton)]
        pub fn set_button(&mut self, button: &str, pressed: bool) -> Result<(), JsValue> {
            self.core
                .set_button(button, pressed)
                .map_err(|error| JsValue::from_str(&error))
        }

        #[wasm_bindgen(js_name = setButtonForPlayer)]
        pub fn set_button_for_player(
            &mut self,
            button: &str,
            pressed: bool,
            player: u8,
        ) -> Result<(), JsValue> {
            self.core
                .set_button_for_player(button, pressed, player)
                .map_err(|error| JsValue::from_str(&error))
        }

        #[wasm_bindgen(js_name = runFrame)]
        pub fn run_frame(&mut self, render_graphics: bool) -> Result<Vec<u8>, JsValue> {
            if render_graphics {
                self.core
                    .run_frame()
                    .map_err(|error| JsValue::from_str(&error))
            } else {
                self.core.run_frame_without_rendering();
                Ok(Vec::new())
            }
        }

        #[wasm_bindgen(js_name = audioSamples)]
        pub fn audio_samples(&self) -> Vec<i16> {
            self.core.audio_samples()
        }

        #[wasm_bindgen(js_name = audioSampleRate)]
        pub fn audio_sample_rate(&self) -> u32 {
            self.core.audio_sample_rate()
        }

        #[wasm_bindgen(js_name = saveState)]
        pub fn save_state(&self) -> Vec<u8> {
            self.core.save_state()
        }

        #[wasm_bindgen(js_name = loadState)]
        pub fn load_state(&mut self, state: &[u8]) -> Result<(), JsValue> {
            self.core
                .load_state(state)
                .map_err(|error| JsValue::from_str(&error.to_string()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_rom(map_mode: u8) -> Vec<u8> {
        let mut rom = vec![0; 1 << 20];
        let header = 0x7F00;
        rom[header + 0xC0..header + 0xD5].copy_from_slice(b"CONTRA3 THE ALIEN WAR");
        rom[header + 0xD5] = map_mode;
        rom[header + 0xD6] = 0;
        rom[header + 0xD7] = 0x0A;
        rom[header + 0xD8] = 0;
        let checksum = 0x1234_u16;
        rom[header + 0xDC..header + 0xDE].copy_from_slice(&(!checksum).to_le_bytes());
        rom[header + 0xDE..header + 0xE0].copy_from_slice(&checksum.to_le_bytes());
        rom
    }

    #[test]
    fn repairs_overlong_lorom_title_without_changing_original_bytes() {
        let rom = test_rom(b'S');
        let repaired = repair_overlong_lorom_title(&rom);

        assert!(matches!(&repaired, Cow::Owned(_)));
        assert_eq!(repaired[0x7FD5], 0x20);
        assert_eq!(rom[0x7FD5], b'S');
    }

    #[test]
    fn leaves_valid_or_inconsistent_headers_unchanged() {
        let valid_rom = test_rom(0x20);
        assert!(matches!(
            repair_overlong_lorom_title(&valid_rom),
            Cow::Borrowed(_)
        ));

        let mut inconsistent_rom = test_rom(b'S');
        inconsistent_rom[0x7FD7] = 0xFF;
        assert!(matches!(
            repair_overlong_lorom_title(&inconsistent_rom),
            Cow::Borrowed(_)
        ));
    }

    #[test]
    fn rejects_unknown_controller_buttons() {
        assert!(button_index("Turbo").is_err());
        assert_eq!(button_index("Start"), Ok(6));
    }

    #[test]
    fn rejects_invalid_rom_data() {
        assert!(SnesCore::new(&[]).is_err());
    }
}
