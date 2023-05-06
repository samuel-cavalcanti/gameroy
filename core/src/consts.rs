/// The current version of GameRoy.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The number of cycles the gameboy runs per second.
pub const CLOCK_SPEED: u64 = 4_194_304;

/// The height of the LCD screen in pixels.
pub const SCREEN_HEIGHT: usize = 144;
/// The width of the LCD screen in pixels.
pub const SCREEN_WIDTH: usize = 160;

/// The number of scanline per frames, including vblank.
pub const SCANLINE_PER_FRAME: u8 = 154;
/// The number of cycles in a single scanline.
pub const SCANLINE_CYCLES: u64 = 456;
/// The number of cycles that a frame have.
pub const FRAME_CYCLES: u64 = SCANLINE_PER_FRAME as u64 * SCANLINE_CYCLES;

pub const IF: usize = 0xff0f;
pub const IE: usize = 0xffff;

/// The length in bytes of each opcode.
#[rustfmt::skip]
pub const LEN: [u8; 256] = [
    1, 3, 1, 1, 1, 1, 2, 1, 3, 1, 1, 1, 1, 1, 2, 1, // 0x
    2, 3, 1, 1, 1, 1, 2, 1, 2, 1, 1, 1, 1, 1, 2, 1, // 1x
    2, 3, 1, 1, 1, 1, 2, 1, 2, 1, 1, 1, 1, 1, 2, 1, // 2x
    2, 3, 1, 1, 1, 1, 2, 1, 2, 1, 1, 1, 1, 1, 2, 1, // 3x
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, // 4x
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, // 5x
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, // 6x
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, // 7x
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, // 8x
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, // 9x
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, // Ax
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, // Bx
    1, 1, 3, 3, 3, 1, 2, 1, 1, 1, 3, 2, 3, 3, 2, 1, // Cx
    1, 1, 3, 1, 3, 1, 2, 1, 1, 1, 3, 1, 3, 1, 2, 1, // Dx
    2, 1, 1, 1, 1, 1, 2, 1, 2, 1, 3, 1, 1, 1, 2, 1, // Ex
    2, 1, 1, 1, 1, 1, 2, 1, 2, 1, 3, 1, 1, 1, 2, 1, // Fx
];

#[rustfmt::skip]
/// The minimum number of clocks that a op takes. May be greater if the operator is taken in the
/// case of jumps, calls and returns.
pub const CLOCK: [u8; 256] = [
     4, 12,  8,  8,  4,  4,  8,  4, 20,  8,  8,  8,  4,  4,  8,  4, // 0x
     4, 12,  8,  8,  4,  4,  8,  4, 12,  8,  8,  8,  4,  4,  8,  4, // 1x
     8, 12,  8,  8,  4,  4,  8,  4,  8,  8,  8,  8,  4,  4,  8,  4, // 2x
     8, 12,  8,  8, 12, 12, 12,  4,  8,  8,  8,  8,  4,  4,  8,  4, // 3x
     4,  4,  4,  4,  4,  4,  8,  4,  4,  4,  4,  4,  4,  4,  8,  4, // 4x
     4,  4,  4,  4,  4,  4,  8,  4,  4,  4,  4,  4,  4,  4,  8,  4, // 5x
     4,  4,  4,  4,  4,  4,  8,  4,  4,  4,  4,  4,  4,  4,  8,  4, // 6x
     8,  8,  8,  8,  8,  8,  4,  8,  4,  4,  4,  4,  4,  4,  8,  4, // 7x
     4,  4,  4,  4,  4,  4,  8,  4,  4,  4,  4,  4,  4,  4,  8,  4, // 8x
     4,  4,  4,  4,  4,  4,  8,  4,  4,  4,  4,  4,  4,  4,  8,  4, // 9x
     4,  4,  4,  4,  4,  4,  8,  4,  4,  4,  4,  4,  4,  4,  8,  4, // Ax
     4,  4,  4,  4,  4,  4,  8,  4,  4,  4,  4,  4,  4,  4,  8,  4, // Bx
     8, 12, 12, 16, 12, 16,  8, 16,  8, 16, 12,  4, 12, 24,  8, 16, // Cx
     8, 12, 12,  4, 12, 16,  8, 16,  8, 16, 12,  4, 12,  4,  8, 16, // Dx
    12, 12,  8,  4,  4, 16,  8, 16, 16,  4, 16,  4,  4,  4,  8, 16, // Ex
    12, 12,  8,  4,  4, 16,  8, 16, 12,  8, 16,  4,  4,  4,  8, 16, // Fx
];

pub const CB_CLOCK: [u8; 256] = [
    8, 8, 8, 8, 8, 8, 16, 8, 8, 8, 8, 8, 8, 8, 16, 8, // 0x
    8, 8, 8, 8, 8, 8, 16, 8, 8, 8, 8, 8, 8, 8, 16, 8, // 1x
    8, 8, 8, 8, 8, 8, 16, 8, 8, 8, 8, 8, 8, 8, 16, 8, // 2x
    8, 8, 8, 8, 8, 8, 16, 8, 8, 8, 8, 8, 8, 8, 16, 8, // 3x
    8, 8, 8, 8, 8, 8, 12, 8, 8, 8, 8, 8, 8, 8, 12, 8, // 4x
    8, 8, 8, 8, 8, 8, 12, 8, 8, 8, 8, 8, 8, 8, 12, 8, // 5x
    8, 8, 8, 8, 8, 8, 12, 8, 8, 8, 8, 8, 8, 8, 12, 8, // 6x
    8, 8, 8, 8, 8, 8, 12, 8, 8, 8, 8, 8, 8, 8, 12, 8, // 7x
    8, 8, 8, 8, 8, 8, 16, 8, 8, 8, 8, 8, 8, 8, 16, 8, // 8x
    8, 8, 8, 8, 8, 8, 16, 8, 8, 8, 8, 8, 8, 8, 16, 8, // 9x
    8, 8, 8, 8, 8, 8, 16, 8, 8, 8, 8, 8, 8, 8, 16, 8, // Ax
    8, 8, 8, 8, 8, 8, 16, 8, 8, 8, 8, 8, 8, 8, 16, 8, // Bx
    8, 8, 8, 8, 8, 8, 16, 8, 8, 8, 8, 8, 8, 8, 16, 8, // Cx
    8, 8, 8, 8, 8, 8, 16, 8, 8, 8, 8, 8, 8, 8, 16, 8, // Dx
    8, 8, 8, 8, 8, 8, 16, 8, 8, 8, 8, 8, 8, 8, 16, 8, // Ex
    8, 8, 8, 8, 8, 8, 16, 8, 8, 8, 8, 8, 8, 8, 16, 8, // Fx
];

const F: bool = false;
const T: bool = true;

/// Instructions that may write in ROM
pub const WRITE_RAM: [bool; 256] = [
    // 1  2  3  4  5  6  7  8  9  A  B  C  D  E  F
    F, F, T, F, F, F, F, F, T, F, F, F, F, F, F, F, // 0x
    F, F, T, F, F, F, F, F, F, F, F, F, F, F, F, F, // 1x
    F, F, T, F, F, F, F, F, F, F, F, F, F, F, F, F, // 2x
    F, F, T, F, T, T, T, F, F, F, F, F, F, F, F, F, // 3x
    F, F, F, F, F, F, F, F, F, F, F, F, F, F, F, F, // 4x
    F, F, F, F, F, F, F, F, F, F, F, F, F, F, F, F, // 5x
    F, F, F, F, F, F, F, F, F, F, F, F, F, F, F, F, // 6x
    T, T, T, T, T, T, F, T, F, F, F, F, F, F, F, F, // 7x
    F, F, F, F, F, F, F, F, F, F, F, F, F, F, F, F, // 8x
    F, F, F, F, F, F, F, F, F, F, F, F, F, F, F, F, // 9x
    F, F, F, F, F, F, F, F, F, F, F, F, F, F, F, F, // Ax
    F, F, F, F, F, F, F, F, F, F, F, F, F, F, F, F, // Bx
    F, F, F, F, T, T, F, T, F, F, F, F, T, T, F, T, // Cx
    F, F, F, F, T, T, F, T, F, F, F, F, T, F, F, T, // Dx
    T, F, T, F, F, T, F, T, F, F, T, F, F, F, F, T, // Ex
    F, F, F, F, F, T, F, T, F, F, F, F, F, F, F, T, // Fx
];
