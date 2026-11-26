// SPDX-License-Identifier: AGPL-3.0-or-later
//! Nearest pixel sampling of the shared screen geometry, with black letterboxing.
//! See backlog/docs/commands.md § Viewing a machine.

use machine_view::{
    screen::{layout, Rect, ScreenLayout},
    Picture,
};
use unemu_core_api::FrameArea;

pub fn geometry(picture: &Picture, width: u32, height: u32) -> Option<ScreenLayout> {
    layout(
        picture,
        Rect {
            x: 0.0,
            y: 0.0,
            width: width as f32,
            height: height as f32,
        },
        false,
        false,
    )
}

/// Everything a painted window's pixels depend on: while it is unchanged the
/// window already shows the right picture, and a paint would redraw it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shown {
    window: (u32, u32),
    picture: Option<ShownPicture>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ShownPicture {
    serial: u64,
    size: (u32, u32),
    aspect: (u32, u32),
    visible: FrameArea,
}

impl Shown {
    pub fn new(picture: Option<&Picture>, width: u32, height: u32) -> Self {
        Self {
            window: (width, height),
            picture: picture.map(|picture| ShownPicture {
                serial: picture.serial,
                size: (picture.width, picture.height),
                aspect: picture.aspect,
                visible: picture.visible,
            }),
        }
    }
}

/// Whether a window of `width` by `height` that last showed `painted` must
/// paint to show `picture`. The platform keeps showing the presented pixels,
/// so an advance that changed nothing shown needs no paint.
pub fn needs_paint(
    painted: Option<Shown>,
    picture: Option<&Picture>,
    width: u32,
    height: u32,
) -> bool {
    painted != Some(Shown::new(picture, width, height))
}

/// The source index of each destination position along one axis whose pixel
/// centre falls inside the shown span: the first such position, then the
/// indices from there on.
///
/// Each index is the pixel-centre fraction `(position + 0.5 - start) / length`
/// in `f32`, scaled to `count` source pixels and truncated, computed once per
/// column and once per row rather than once per pixel. The covered positions
/// are contiguous because the fraction never decreases along the axis
/// (rounding keeps that order).
fn samples(start: f32, length: f32, first: u32, count: u32, limit: u32) -> (usize, Vec<u32>) {
    let mut from = None;
    let mut indices = Vec::new();
    for position in (start.floor() as u32)..((start + length).ceil() as u32).min(limit) {
        let fraction = (position as f32 + 0.5 - start) / length;
        if (0.0..1.0).contains(&fraction) {
            from.get_or_insert(position as usize);
            indices.push(first + (fraction * count as f32) as u32);
        } else if from.is_some() {
            break;
        }
    }
    (from.unwrap_or(0), indices)
}

pub fn blit(picture: Option<&Picture>, width: u32, height: u32, pixels: &mut [u32]) {
    let Some((picture, shown)) = picture.and_then(|p| geometry(p, width, height).map(|s| (p, s)))
    else {
        pixels.fill(0);
        return;
    };
    let (dest, source) = (shown.dest, shown.source);
    let (left, columns) = samples(dest.x, dest.width, source.x, source.width, width);
    let (top, rows) = samples(dest.y, dest.height, source.y, source.height, height);
    let width = width as usize;
    let right = left + columns.len();
    let bottom = top + rows.len();
    // Only the letterbox needs clearing: the loop below writes every pixel
    // of the shown rows, and clearing the whole window first would write a
    // large buffer twice on every paint.
    pixels[..top * width].fill(0);
    pixels[bottom * width..].fill(0);
    let mut previous: Option<(u32, usize)> = None;
    for (y, &sy) in (top..bottom).zip(&rows) {
        let line = y * width;
        match previous {
            // A scaled-up picture repeats each source row on several window
            // rows; copying the finished row is much cheaper than sampling.
            Some((last, from)) if last == sy => pixels.copy_within(from..from + width, line),
            _ => {
                let row = &mut pixels[line..line + width];
                row[..left].fill(0);
                row[right..].fill(0);
                let base = (sy * picture.width) as usize;
                for (pixel, &sx) in row[left..right].iter_mut().zip(&columns) {
                    *pixel = picture.pixels[base + sx as usize];
                }
            }
        }
        previous = Some((sy, line));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn picture() -> Picture {
        Picture {
            width: 2,
            height: 2,
            pixels: vec![1, 2, 3, 4],
            aspect: (4, 3),
            serial: 0,
            visible: FrameArea::new(0, 0, 2, 2),
        }
    }

    /// Per-pixel `f32` centre sampling: the specification the precomputed
    /// blit must equal pixel for pixel.
    fn reference_blit(picture: Option<&Picture>, width: u32, height: u32, pixels: &mut [u32]) {
        pixels.fill(0);
        let Some((picture, shown)) =
            picture.and_then(|p| geometry(p, width, height).map(|s| (p, s)))
        else {
            return;
        };
        let dest = shown.dest;
        let source = shown.source;
        for y in (dest.y.floor() as u32)..((dest.y + dest.height).ceil() as u32).min(height) {
            let v = (y as f32 + 0.5 - dest.y) / dest.height;
            if !(0.0..1.0).contains(&v) {
                continue;
            }
            let sy = source.y + (v * source.height as f32) as u32;
            for x in (dest.x.floor() as u32)..((dest.x + dest.width).ceil() as u32).min(width) {
                let u = (x as f32 + 0.5 - dest.x) / dest.width;
                if !(0.0..1.0).contains(&u) {
                    continue;
                }
                let sx = source.x + (u * source.width as f32) as u32;
                pixels[(y * width + x) as usize] =
                    picture.pixels[(sy * picture.width + sx) as usize];
            }
        }
    }

    /// A picture whose pixels all differ, so a sample of the wrong source
    /// pixel always shows.
    fn numbered(width: u32, height: u32, aspect: (u32, u32), visible: FrameArea) -> Picture {
        Picture {
            width,
            height,
            pixels: (1..=width * height).collect(),
            aspect,
            serial: 0,
            visible,
        }
    }

    #[test]
    fn aspect_letterbox_and_nearest_samples() {
        let picture = picture();
        let shown = geometry(&picture, 8, 8).unwrap();
        assert_eq!(
            shown.dest,
            Rect {
                x: 0.0,
                y: 1.0,
                width: 8.0,
                height: 6.0
            }
        );
        let mut pixels = vec![99; 64];
        blit(Some(&picture), 8, 8, &mut pixels);
        assert_eq!(&pixels[..8], &[0; 8]);
        assert_eq!(&pixels[8..16], &[1, 1, 1, 1, 2, 2, 2, 2]);
        assert_eq!(&pixels[48..56], &[3, 3, 3, 3, 4, 4, 4, 4]);
        assert_eq!(&pixels[56..], &[0; 8]);
        assert!(geometry(&picture, 0, 8).is_none());
    }

    #[test]
    fn crops_overscan_and_handles_downscaling_and_blank_picture() {
        let mut picture = picture();
        picture.visible = FrameArea::new(1, 0, 1, 2);
        let mut pixels = vec![99; 6];
        blit(Some(&picture), 2, 3, &mut pixels);
        assert_eq!(pixels, [2, 2, 4, 4, 4, 4]);
        picture.visible = FrameArea::new(0, 0, 2, 2);
        picture.aspect = (1, 1);
        blit(Some(&picture), 1, 1, &mut pixels[..1]);
        assert_eq!(pixels[0], 4);
        blit(None, 2, 3, &mut pixels);
        assert_eq!(pixels, [0; 6]);
    }

    #[test]
    fn matches_per_pixel_sampling_for_every_window_and_picture_shape() {
        let pictures = [
            numbered(320, 200, (4, 3), FrameArea::new(0, 0, 320, 200)),
            numbered(640, 400, (4, 3), FrameArea::new(0, 0, 640, 400)),
            numbered(720, 400, (4, 3), FrameArea::new(0, 0, 720, 400)),
            numbered(640, 480, (4, 3), FrameArea::new(0, 0, 640, 480)),
            // Cropped overscan, odd sizes, and shapes taller than wide.
            numbered(341, 263, (4, 3), FrameArea::new(13, 7, 301, 241)),
            numbered(97, 131, (3, 4), FrameArea::new(0, 0, 97, 131)),
            numbered(5, 3, (16, 9), FrameArea::new(1, 1, 3, 1)),
            numbered(1, 1, (1, 1), FrameArea::new(0, 0, 1, 1)),
            // A picture with no usable shape: the whole window is cleared.
            numbered(4, 3, (0, 1), FrameArea::new(0, 0, 4, 3)),
        ];
        let windows = [
            (1280, 800),
            (2560, 1600),
            (1920, 1440),
            (960, 720),
            (1, 1),
            (3, 1000),
            (1000, 3),
            (321, 201),
            (639, 479),
            (1001, 777),
            (2049, 1537),
            (160, 100),
            (77, 999),
        ];
        for picture in &pictures {
            for &(width, height) in &windows {
                let size = (width * height) as usize;
                let mut expected = vec![0xdead_beef; size];
                let mut actual = vec![0xdead_beef; size];
                reference_blit(Some(picture), width, height, &mut expected);
                blit(Some(picture), width, height, &mut actual);
                assert!(
                    expected == actual,
                    "{}x{} picture, {width}x{height} window",
                    picture.width,
                    picture.height
                );
            }
        }
    }

    #[test]
    fn paints_only_when_something_shown_changed() {
        let picture = numbered(4, 3, (4, 3), FrameArea::new(0, 0, 4, 3));
        assert!(needs_paint(None, Some(&picture), 800, 600));
        assert!(needs_paint(None, None, 800, 600));
        let painted = Some(Shown::new(Some(&picture), 800, 600));
        assert!(!needs_paint(painted, Some(&picture), 800, 600));
        assert!(needs_paint(painted, Some(&picture), 800, 601));
        assert!(needs_paint(painted, None, 800, 600));
        let mut redrawn = picture.clone();
        redrawn.serial += 1;
        assert!(needs_paint(painted, Some(&redrawn), 800, 600));
    }

    #[test]
    fn shown_state_changes_with_anything_the_window_shows() {
        let picture = numbered(4, 3, (4, 3), FrameArea::new(0, 0, 4, 3));
        let shown = Shown::new(Some(&picture), 800, 600);
        assert_eq!(shown, Shown::new(Some(&picture.clone()), 800, 600));
        assert_ne!(shown, Shown::new(None, 800, 600));
        assert_ne!(shown, Shown::new(Some(&picture), 801, 600));
        assert_ne!(shown, Shown::new(Some(&picture), 800, 601));
        let changed = |change: fn(&mut Picture)| {
            let mut changed = picture.clone();
            change(&mut changed);
            Shown::new(Some(&changed), 800, 600)
        };
        assert_ne!(shown, changed(|p| p.serial += 1));
        assert_ne!(shown, changed(|p| p.aspect = (16, 9)));
        assert_ne!(shown, changed(|p| p.visible = FrameArea::new(1, 0, 3, 3)));
        assert_ne!(
            shown,
            changed(|p| {
                p.width = 3;
                p.height = 4;
            })
        );
    }
}
