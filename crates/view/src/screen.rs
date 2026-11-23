// SPDX-License-Identifier: AGPL-3.0-or-later
//! Nearest pixel sampling of the shared screen geometry, with black letterboxing.
//! See backlog/docs/commands.md § Viewing a machine.

use machine_view::{
    screen::{layout, Rect, ScreenLayout},
    Picture,
};

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

pub fn blit(picture: Option<&Picture>, width: u32, height: u32, pixels: &mut [u32]) {
    pixels.fill(0);
    let Some((picture, shown)) = picture.and_then(|p| geometry(p, width, height).map(|s| (p, s)))
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
            pixels[(y * width + x) as usize] = picture.pixels[(sy * picture.width + sx) as usize];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use unemu_core_api::FrameArea;

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
}
