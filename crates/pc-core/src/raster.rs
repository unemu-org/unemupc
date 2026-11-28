// SPDX-License-Identifier: AGPL-3.0-or-later
//! Pure text-cell, indexed-pixel and direct-colour rasterisation
//! (design/spec.md §9.4).
//!
//! IBM PC Technical Reference, Monochrome Display and Printer Adapter,
//! Programming Considerations, defines MDA attribute classes; IBM VGA Technical
//! Reference, Alphanumeric Modes, defines foreground/background palette indices.
//! This presenter consumes decoded styles, never raw card registers.

use crate::{FontAtlas, FontError, MDA_FONT, VGA_FONT};
use std::{error::Error, fmt, num::NonZeroU16};
use video::{MonochromeEffect, TextGrid, TextStyle};

const PALETTE: [u32; 16] = [
    0x000000, 0x0000AA, 0x00AA00, 0x00AAAA, 0xAA0000, 0xAA00AA, 0xAA5500, 0xAAAAAA, 0x555555,
    0x5555FF, 0x55FF55, 0x55FFFF, 0xFF5555, 0xFF55FF, 0xFFFF55, 0xFFFFFF,
];

/// A complete row-major RGB image (`0x00RRGGBB`) for host texture upload.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PixelFrame {
    /// Width in pixels, including integer scale.
    pub width: u32,
    /// Height in pixels, including integer scale.
    pub height: u32,
    /// Exactly `width * height` pixels.
    pub pixels: Vec<u32>,
}

/// A frame that cannot fit the presenter's bounded pixel buffer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RasterError {
    /// Frame exceeds 16,777,216 pixels (64 MiB) or dimensions overflow.
    TooLarge,
    /// The indexed buffer does not contain exactly width × height pixels, or
    /// a direct-colour buffer ends before its last visible pixel.
    InvalidPixelCount,
    /// A pixel selects an entry absent from its captured palette.
    PaletteIndexOutOfRange,
    /// A presentation repetition factor is zero.
    InvalidRepeat,
    /// A direct-colour scan-line stride is shorter than one row of pixels.
    InvalidStride,
}
impl fmt::Display for RasterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::TooLarge => "frame exceeds the 64 MiB pixel buffer limit",
            Self::InvalidPixelCount => "pixel count does not match frame dimensions",
            Self::PaletteIndexOutOfRange => "pixel index is outside the frame palette",
            Self::InvalidRepeat => "pixel repetition factors must be nonzero",
            Self::InvalidStride => "scan-line stride is shorter than one row of pixels",
        })
    }
}
impl Error for RasterError {}

/// CP437 rasteriser with bundled monochrome and colour font atlases.
pub struct Rasterizer {
    mda: FontAtlas,
    vga: FontAtlas,
    scale: NonZeroU16,
}

impl Rasterizer {
    /// Load the original MDA 9×14 and VGA 8×16 fonts once.
    pub fn bundled(scale: NonZeroU16) -> Result<Self, FontError> {
        Ok(Self {
            mda: FontAtlas::from_fon(MDA_FONT)?,
            vga: FontAtlas::from_fon(VGA_FONT)?,
            scale,
        })
    }

    /// Convert any frame kind into an integer-scaled host buffer.
    /// Indexed graphics use their captured DAC palette, without host time.
    pub fn rasterize_frame(&self, frame: &video::Frame) -> Result<PixelFrame, RasterError> {
        let source = match frame {
            video::Frame::Text(grid) => return self.rasterize(grid),
            video::Frame::Direct(source) => return self.rasterize_direct(source),
            video::Frame::Pixels(source) => source,
        };
        if source.indices.len() != usize::from(source.width) * usize::from(source.height) {
            return Err(RasterError::InvalidPixelCount);
        }
        if source
            .indices
            .iter()
            .any(|&index| usize::from(index) >= source.palette.len())
        {
            return Err(RasterError::PaletteIndexOutOfRange);
        }
        if source.repeat.0 == 0 || source.repeat.1 == 0 {
            return Err(RasterError::InvalidRepeat);
        }
        let scale = u32::from(self.scale.get());
        let scale_x = scale * u32::from(source.repeat.0);
        let scale_y = scale * u32::from(source.repeat.1);
        let width = u32::from(source.width)
            .checked_mul(scale_x)
            .ok_or(RasterError::TooLarge)?;
        let height = u32::from(source.height)
            .checked_mul(scale_y)
            .ok_or(RasterError::TooLarge)?;
        let len = u64::from(width) * u64::from(height);
        if len > 16_777_216 {
            return Err(RasterError::TooLarge);
        }
        let mut pixels = vec![0; len as usize];
        for y in 0..height {
            for x in 0..width {
                let index = source.indices
                    [((y / scale_y) * u32::from(source.width) + x / scale_x) as usize];
                let [r, g, b] = source.palette[usize::from(index)];
                pixels[(y * width + x) as usize] =
                    (u32::from(r) << 16) | (u32::from(g) << 8) | u32::from(b);
            }
        }
        Ok(PixelFrame {
            width,
            height,
            pixels,
        })
    }

    /// Convert a direct-colour frame with the same validation, repetition and
    /// integer scaling as indexed graphics. The buffer layout and the output
    /// size are checked before any pixel is converted.
    fn rasterize_direct(&self, source: &video::DirectPixels) -> Result<PixelFrame, RasterError> {
        source.check_layout().map_err(layout_error)?;
        if source.repeat.0 == 0 || source.repeat.1 == 0 {
            return Err(RasterError::InvalidRepeat);
        }
        let scale = u32::from(self.scale.get());
        let scale_x = scale * u32::from(source.repeat.0);
        let scale_y = scale * u32::from(source.repeat.1);
        let width = u32::from(source.width)
            .checked_mul(scale_x)
            .ok_or(RasterError::TooLarge)?;
        let height = u32::from(source.height)
            .checked_mul(scale_y)
            .ok_or(RasterError::TooLarge)?;
        let len = u64::from(width) * u64::from(height);
        if len > 16_777_216 {
            return Err(RasterError::TooLarge);
        }
        let logical = source.to_xrgb8888().map_err(layout_error)?;
        if (scale_x, scale_y) == (1, 1) || len == 0 {
            return Ok(PixelFrame {
                width,
                height,
                pixels: logical,
            });
        }
        // Widen each logical row once, then copy the finished row for the
        // remaining repeated scan lines.
        let row = width as usize;
        let mut pixels = Vec::with_capacity(len as usize);
        for line in logical.chunks_exact(usize::from(source.width)) {
            let start = pixels.len();
            for &pixel in line {
                pixels.extend(std::iter::repeat_n(pixel, scale_x as usize));
            }
            for _ in 1..scale_y {
                pixels.extend_from_within(start..start + row);
            }
        }
        Ok(PixelFrame {
            width,
            height,
            pixels,
        })
    }

    /// Rasterise a grid without a window or host time. One card supplies one
    /// attribute set; the first cell selects its font (empty grids use MDA).
    /// Blink stays visible. Integer scaling replicates pixels without filtering.
    pub fn rasterize(&self, grid: &TextGrid) -> Result<PixelFrame, RasterError> {
        let mono = grid
            .cell(0, 0)
            .is_none_or(|cell| matches!(cell.style, TextStyle::Monochrome { .. }));
        let font = if mono { &self.mda } else { &self.vga };
        let scale = u32::from(self.scale.get());
        let appearance = grid.appearance();
        let dot_width = appearance.map_or(font.width(), |a| u16::from(a.cell_width));
        let dot_height = appearance.map_or(font.height(), |a| u16::from(a.cell_height));
        let horizontal_scale = scale * appearance.map_or(1, |a| u32::from(a.horizontal_repeat));
        let cell_width = u32::from(dot_width) * horizontal_scale;
        let cell_height = u32::from(dot_height) * scale;
        let width = u32::from(grid.cols())
            .checked_mul(cell_width)
            .ok_or(RasterError::TooLarge)?;
        let height = u32::from(grid.rows())
            .checked_mul(cell_height)
            .ok_or(RasterError::TooLarge)?;
        let len = u64::from(width) * u64::from(height);
        if len > 16_777_216 {
            return Err(RasterError::TooLarge);
        }
        let mut image = PixelFrame {
            width,
            height,
            pixels: vec![0; len as usize],
        };
        let cursor = grid.cursor();
        for row in 0..grid.rows() {
            for col in 0..grid.cols() {
                let Some(cell) = grid.cell(col, row) else {
                    continue;
                };
                let (mut fg, mut bg, mut underline) = colours(cell.style);
                if let Some(a) = appearance {
                    if let TextStyle::Colour {
                        foreground,
                        background,
                        ..
                    } = cell.style
                    {
                        let rgb = |index: u8| {
                            let [r, g, b] = a.palette[usize::from(index)];
                            (u32::from(r) << 16) | (u32::from(g) << 8) | u32::from(b)
                        };
                        fg = rgb(foreground);
                        bg = rgb(background);
                        underline = cell.attr & 0x77 == 1;
                    }
                    if !a.enabled {
                        fg = 0;
                        bg = 0;
                    }
                }
                let cursor_here = cursor.visible && cursor.col == col && cursor.row == row;
                for y in 0..dot_height {
                    let bits = grid
                        .glyph_row(cell, y as u8)
                        .unwrap_or_else(|| font.glyph_rows(cell.glyph)[usize::from(y)]);
                    let cursor_start = appearance
                        .map_or(if mono { 11 } else { 13 }, |a| u16::from(a.cursor_start));
                    let cursor_end =
                        appearance.map_or(cursor_start + 1, |a| u16::from(a.cursor_end));
                    let underline_row = appearance.map_or(12, |a| u16::from(a.underline));
                    let line = (underline && y == underline_row)
                        || (cursor_here && (cursor_start..=cursor_end).contains(&y));
                    for x in 0..dot_width {
                        let lit = line || bits & (1 << (dot_width - 1 - x)) != 0;
                        let pixel = if lit { fg } else { bg };
                        let px = u32::from(col) * cell_width + u32::from(x) * horizontal_scale;
                        let py = u32::from(row) * cell_height + y as u32 * scale;
                        for dy in 0..scale {
                            let start = ((py + dy) * width + px) as usize;
                            image.pixels[start..start + horizontal_scale as usize].fill(pixel);
                        }
                    }
                }
            }
        }
        Ok(image)
    }
}

fn layout_error(error: video::DirectLayoutError) -> RasterError {
    match error {
        video::DirectLayoutError::StrideTooShort => RasterError::InvalidStride,
        video::DirectLayoutError::BufferTooShort => RasterError::InvalidPixelCount,
    }
}

fn colours(style: TextStyle) -> (u32, u32, bool) {
    match style {
        TextStyle::Colour {
            foreground,
            background,
            ..
        } => (
            PALETTE[usize::from(foreground & 15)],
            PALETTE[usize::from(background & 15)],
            false,
        ),
        TextStyle::Monochrome {
            effect,
            intense,
            background_intense,
            ..
        } => {
            let ink = if intense { 0xFFFFFF } else { 0xAAAAAA };
            let paper = if background_intense { 0x555555 } else { 0 };
            match effect {
                MonochromeEffect::NonDisplay => (0, 0, false),
                MonochromeEffect::Normal => (ink, paper, false),
                MonochromeEffect::Underline => (ink, paper, true),
                MonochromeEffect::Reverse => (
                    0,
                    if background_intense {
                        0xFFFFFF
                    } else {
                        0xAAAAAA
                    },
                    false,
                ),
            }
        }
    }
}

#[cfg(test)]
mod planar_tests {
    use super::*;
    use video::{render_planar_text, PlanarTextState};

    #[test]
    fn pixel_frames_validate_palette_range_repeat_and_scaled_bounds() {
        let raster = Rasterizer::bundled(NonZeroU16::new(1).unwrap()).unwrap();
        let mut image = video::IndexedPixels {
            width: 1,
            height: 1,
            repeat: (1, 1),
            indices: vec![2],
            palette: vec![[0; 3], [1; 3]].into_boxed_slice(),
        };
        assert_eq!(
            raster.rasterize_frame(&video::Frame::Pixels(image.clone())),
            Err(RasterError::PaletteIndexOutOfRange)
        );
        image.indices[0] = 1;
        assert!(raster
            .rasterize_frame(&video::Frame::Pixels(image.clone()))
            .is_ok());
        for repeat in [(0, 1), (1, 0)] {
            image.repeat = repeat;
            assert_eq!(
                raster.rasterize_frame(&video::Frame::Pixels(image.clone())),
                Err(RasterError::InvalidRepeat)
            );
        }
        image.repeat = (255, 255);
        let large_scale = Rasterizer::bundled(NonZeroU16::new(u16::MAX).unwrap()).unwrap();
        assert_eq!(
            large_scale.rasterize_frame(&video::Frame::Pixels(image.clone())),
            Err(RasterError::TooLarge)
        );
        image.width = u16::MAX;
        image.indices.resize(usize::from(u16::MAX), 0);
        assert_eq!(
            large_scale.rasterize_frame(&video::Frame::Pixels(image)),
            Err(RasterError::TooLarge)
        );
        let overflow_scale = Rasterizer::bundled(NonZeroU16::new(32768).unwrap()).unwrap();
        let overflow = video::Frame::Pixels(video::IndexedPixels {
            width: 32768,
            height: 1,
            repeat: (4, 1),
            indices: vec![0; 32768],
            palette: vec![[0; 3]].into_boxed_slice(),
        });
        assert_eq!(
            overflow_scale.rasterize_frame(&overflow),
            Err(RasterError::TooLarge)
        );
    }

    #[test]
    fn graphics_repeat_factors_restore_monitor_sizes_before_integer_scaling() {
        for scale in [1, 2] {
            let raster = Rasterizer::bundled(NonZeroU16::new(scale).unwrap()).unwrap();
            for (mode, width, height, repeat, displayed) in [
                (0x0d, 320, 200, (2, 2), (640, 400)),
                (0x0e, 640, 200, (1, 2), (640, 400)),
                (0x12, 640, 480, (1, 1), (640, 480)),
                (0x13, 320, 200, (2, 2), (640, 400)),
            ] {
                let frame = video::Frame::Pixels(video::IndexedPixels {
                    width,
                    height,
                    repeat,
                    indices: vec![0; usize::from(width) * usize::from(height)],
                    palette: vec![[0; 3]].into_boxed_slice(),
                });
                let pixels = raster.rasterize_frame(&frame).unwrap();
                assert_eq!(
                    (pixels.width, pixels.height),
                    (
                        displayed.0 * u32::from(scale),
                        displayed.1 * u32::from(scale)
                    ),
                    "mode {mode:02x}"
                );
            }
        }
        let grid = video::render_text_mode(&video::TextModeState {
            vram: &[],
            cols: 80,
            rows: 25,
            start_cell: 0,
            cursor_cell: 0,
            cursor_visible: false,
            attributes: video::TextAttributeSet::Colour {
                blink_enabled: true,
            },
        });
        let raster = Rasterizer::bundled(NonZeroU16::new(1).unwrap()).unwrap();
        let pixels = raster.rasterize_frame(&video::Frame::Text(grid)).unwrap();
        assert_eq!((pixels.width, pixels.height), (640, 400));
        let mut palette = vec![[0; 3]; 2];
        palette[1] = [255, 0, 0];
        let pixels = raster
            .rasterize_frame(&video::Frame::Pixels(video::IndexedPixels {
                width: 2,
                height: 1,
                repeat: (2, 3),
                indices: vec![0, 1],
                palette: palette.into_boxed_slice(),
            }))
            .unwrap();
        assert_eq!(pixels.pixels, [0, 0, 0xff0000, 0xff0000].repeat(3));
    }

    fn direct_frame(width: u16, height: u16, stride: u32, bytes: Vec<u8>) -> video::Frame {
        video::Frame::Direct(video::DirectPixels {
            width,
            height,
            repeat: (1, 1),
            format: video::PixelFormat::Rgb565,
            stride,
            bytes,
        })
    }

    #[test]
    fn direct_colour_frame_reaches_the_host_buffer_with_repeat_and_scale() {
        // Rgb565 red and a mid value, then blue and white; two padding bytes per row.
        let bytes = vec![
            0x00, 0xF8, 0x01, 0x84, 0xAA, 0xAA, //
            0x1F, 0x00, 0xFF, 0xFF, 0xAA, 0xAA,
        ];
        let video::Frame::Direct(mut source) = direct_frame(2, 2, 6, bytes) else {
            unreachable!()
        };
        source.repeat = (2, 1);
        let frame = video::Frame::Direct(source);
        let mut sink = crate::RasterSink::new(NonZeroU16::new(2).unwrap()).unwrap();
        video::FrameSink::present(&mut sink, &frame);
        let top = [[0xFF0000; 4], [0x848208; 4]].concat();
        let bottom = [[0x0000FF; 4], [0xFFFFFF; 4]].concat();
        assert_eq!(
            *sink.frame(),
            Some(PixelFrame {
                width: 8,
                height: 4,
                pixels: [top.clone(), top, bottom.clone(), bottom].concat(),
            })
        );
        assert_eq!(sink.error(), None);
    }

    #[test]
    fn direct_colour_unscaled_empty_and_unevenly_repeated_frames_are_exact() {
        let raster = Rasterizer::bundled(NonZeroU16::new(1).unwrap()).unwrap();
        // Rgb565 red, blue; the last row ends at its last pixel.
        let bytes = vec![0x00, 0xF8, 0x1F, 0x00];
        assert_eq!(
            raster.rasterize_frame(&direct_frame(2, 1, 4, bytes.clone())),
            Ok(PixelFrame {
                width: 2,
                height: 1,
                pixels: vec![0xFF0000, 0x0000FF],
            })
        );
        for (width, height) in [(0, 3), (3, 0)] {
            let video::Frame::Direct(mut source) = direct_frame(width, height, 0, Vec::new())
            else {
                unreachable!()
            };
            source.repeat = (2, 2);
            assert_eq!(
                raster
                    .rasterize_frame(&video::Frame::Direct(source))
                    .map(|frame| frame.pixels.len()),
                Ok(0)
            );
        }
        let video::Frame::Direct(mut source) = direct_frame(2, 1, 4, bytes) else {
            unreachable!()
        };
        source.repeat = (3, 2);
        let row = [[0xFF0000; 3], [0x0000FF; 3]].concat();
        assert_eq!(
            raster.rasterize_frame(&video::Frame::Direct(source)),
            Ok(PixelFrame {
                width: 6,
                height: 2,
                pixels: [row.clone(), row].concat(),
            })
        );
    }

    #[test]
    fn direct_colour_frames_validate_stride_length_repeat_and_size() {
        let raster = Rasterizer::bundled(NonZeroU16::new(1).unwrap()).unwrap();
        assert_eq!(
            raster.rasterize_frame(&direct_frame(2, 1, 3, vec![0; 3])),
            Err(RasterError::InvalidStride)
        );
        assert_eq!(
            raster.rasterize_frame(&direct_frame(2, 1, 4, vec![0; 3])),
            Err(RasterError::InvalidPixelCount)
        );
        assert!(raster
            .rasterize_frame(&direct_frame(2, 1, 4, vec![0; 5]))
            .is_ok());
        for repeat in [(0, 1), (1, 0)] {
            let video::Frame::Direct(mut source) = direct_frame(1, 1, 2, vec![0; 2]) else {
                unreachable!()
            };
            source.repeat = repeat;
            assert_eq!(
                raster.rasterize_frame(&video::Frame::Direct(source)),
                Err(RasterError::InvalidRepeat)
            );
        }
        // Scaled to 4096 × 4096 a pixel just fits the 16,777,216-pixel host
        // limit; one more repeated dot does not.
        let at_limit = Rasterizer::bundled(NonZeroU16::new(4096).unwrap()).unwrap();
        assert!(at_limit
            .rasterize_frame(&direct_frame(1, 1, 2, vec![0; 2]))
            .is_ok());
        assert_eq!(
            at_limit.rasterize_frame(&direct_frame(2, 1, 4, vec![0; 4])),
            Err(RasterError::TooLarge)
        );
        let overflow_scale = Rasterizer::bundled(NonZeroU16::new(32768).unwrap()).unwrap();
        let video::Frame::Direct(mut source) = direct_frame(32768, 1, 65536, vec![0; 65536]) else {
            unreachable!()
        };
        source.repeat = (4, 1);
        assert_eq!(
            overflow_scale.rasterize_frame(&video::Frame::Direct(source)),
            Err(RasterError::TooLarge)
        );
    }

    #[test]
    fn malformed_pixel_counts_return_an_error() {
        let raster = Rasterizer::bundled(NonZeroU16::new(1).unwrap()).unwrap();
        for indices in [vec![], vec![0; 3], vec![0; 5]] {
            let frame = video::Frame::Pixels(video::IndexedPixels {
                width: 2,
                height: 2,
                repeat: (1, 1),
                indices,
                palette: Box::new([[0; 3]; 256]),
            });
            assert!(raster.rasterize_frame(&frame).is_err());
        }
    }

    #[test]
    fn indexed_frame_palette_and_integer_scaling_reach_the_host_buffer() {
        let mut palette = [[0; 3]; 256];
        palette[1] = [1, 2, 3];
        palette[200] = [255, 128, 16];
        let frame = video::Frame::Pixels(video::IndexedPixels {
            width: 2,
            height: 2,
            repeat: (1, 1),
            indices: vec![1, 200, 0, 1],
            palette: Box::new(palette),
        });
        let mut sink = crate::RasterSink::new(NonZeroU16::new(2).unwrap()).unwrap();
        video::FrameSink::present(&mut sink, &frame);
        assert_eq!(
            *sink.frame(),
            Some(PixelFrame {
                width: 4,
                height: 4,
                pixels: vec![
                    0x010203, 0x010203, 0xff8010, 0xff8010, 0x010203, 0x010203, 0xff8010, 0xff8010,
                    0, 0, 0x010203, 0x010203, 0, 0, 0x010203, 0x010203
                ],
            })
        );
        assert_eq!(sink.error(), None);
        let mut capture = video::CaptureFrameSink::new();
        video::FrameSink::present(&mut capture, &frame);
        assert_eq!(capture.last(), Some(&frame));
        assert_eq!(capture.last_text(), None);
    }

    #[test]
    fn guest_font_palette_ninth_dot_cursor_underline_and_blank_are_presented() {
        let glyphs = vec![0xC0, b'A'];
        let attributes = vec![2, 11];
        let mut font = vec![0; 65536];
        font[0xC0 * 32] = 1;
        font[0x2000 + usize::from(b'A') * 32] = 0x80;
        let mut palette = [[0; 3]; 16];
        palette[1] = [0, 255, 0];
        palette[2] = [255, 0, 0];
        palette[3] = [0, 0, 255];
        palette[11] = [0, 255, 0];
        let mut state = PlanarTextState {
            scroll: Default::default(),
            scan_lines: 2,
            planes: [&glyphs, &attributes, &font],
            dimensions: (2, 1),
            start: 0,
            cursor: 1,
            row_stride: 2,
            address_shift: 0,
            word_wrap_bit: 15,
            font_offsets: [0, 0x2000],
            dual_font: true,
            cell_size: (9, 2),
            horizontal_repeat: 1,
            line_graphics: true,
            blink_enabled: true,
            cursor_shape: (true, 1, 1),
            underline: 1,
            palette,
            enabled: true,
        };
        let raster = Rasterizer::bundled(NonZeroU16::new(1).unwrap()).unwrap();
        let pixels = raster.rasterize(&render_planar_text(&state)).unwrap();
        let mut expected = vec![0; 36];
        expected[7..9].fill(0xFF0000);
        expected[9] = 0x00FF00;
        expected[27..36].fill(0x00FF00);
        assert_eq!(
            pixels,
            PixelFrame {
                width: 18,
                height: 2,
                pixels: expected
            }
        );
        state.enabled = false;
        assert_eq!(
            raster
                .rasterize(&render_planar_text(&state))
                .unwrap()
                .pixels,
            vec![0; 36]
        );
        let underlined_attributes = vec![1, 11];
        state.planes[1] = &underlined_attributes;
        state.enabled = true;
        state.cursor_shape.0 = false;
        let pixels = raster.rasterize(&render_planar_text(&state)).unwrap();
        assert_eq!(&pixels.pixels[18..27], &[0x00FF00; 9]);
        assert_eq!(&pixels.pixels[27..], &[0; 9]);
    }

    #[test]
    fn divided_dot_clock_doubles_each_dot_for_forty_columns() {
        let glyphs = vec![b'A'; 40];
        let attributes = vec![2; 40];
        let mut font = vec![0; 65536];
        font[usize::from(b'A') * 32] = 0x81;
        let mut palette = [[0; 3]; 16];
        palette[2] = [255, 0, 0];
        let state = PlanarTextState {
            scroll: Default::default(),
            scan_lines: 2,
            planes: [&glyphs, &attributes, &font],
            dimensions: (40, 1),
            start: 0,
            cursor: 0,
            row_stride: 40,
            address_shift: 0,
            word_wrap_bit: 15,
            font_offsets: [0, 0],
            dual_font: false,
            cell_size: (9, 2),
            horizontal_repeat: 2,
            line_graphics: false,
            blink_enabled: true,
            cursor_shape: (false, 0, 0),
            underline: 31,
            palette,
            enabled: true,
        };
        let raster = Rasterizer::bundled(NonZeroU16::new(1).unwrap()).unwrap();
        let pixels = raster.rasterize(&render_planar_text(&state)).unwrap();
        let mut expected_row = [0; 18];
        expected_row[0..2].fill(0xFF0000);
        expected_row[14..16].fill(0xFF0000);
        let mut expected = expected_row.repeat(40);
        expected.extend(vec![0; 720]);
        assert_eq!(
            pixels,
            PixelFrame {
                width: 720,
                height: 2,
                pixels: expected
            }
        );
    }
}
