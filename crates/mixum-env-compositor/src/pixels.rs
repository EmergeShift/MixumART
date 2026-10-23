//! Per-frame pixel helpers for the B-route compositor.
//!
//! The compositor itself only builds on Linux, but the hot loops below are pure
//! buffer arithmetic. Keeping them in a platform-independent module means the
//! same code can be unit-tested and benchmarked on the machines we develop on
//! (`cargo test -p mixum-env-compositor --release -- --ignored --nocapture`),
//! instead of only inside a container.

use png::{BitDepth, ColorType, Compression, Encoder, FilterType};

/// One `wl_shm` surface buffer, converted to tightly packed RGBA8888.
#[derive(Clone)]
pub struct SurfacePixels {
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<u8>,
    /// True when the source format was `Xrgb8888`: every pixel is opaque and
    /// the blitter can copy rows instead of blending them.
    pub opaque: bool,
}

/// Convert little-endian `BGRA` (`Argb8888`) or `BGRX` (`Xrgb8888`) rows into
/// tightly packed RGBA8888.
pub fn bgra_to_rgba(
    source: &[u8],
    stride: usize,
    width: usize,
    height: usize,
    opaque: bool,
) -> SurfacePixels {
    debug_assert!(source.len() >= stride * height);
    let mut rgba = vec![0u8; width * height * 4];
    let row_bytes = width * 4;
    for (row, dst) in rgba.chunks_exact_mut(row_bytes).enumerate() {
        let src = &source[row * stride..row * stride + row_bytes];
        for offset in (0..row_bytes).step_by(4) {
            dst[offset] = src[offset + 2];
            dst[offset + 1] = src[offset + 1];
            dst[offset + 2] = src[offset];
            dst[offset + 3] = if opaque { 255 } else { src[offset + 3] };
        }
    }
    SurfacePixels {
        width,
        height,
        rgba,
        opaque,
    }
}

/// RGBA8888 canvas that surface buffers are composited into.
pub struct Canvas {
    pixels: Vec<u8>,
    width: usize,
    height: usize,
}

impl Canvas {
    pub fn new(width: usize, height: usize) -> Self {
        Self {
            pixels: vec![0u8; width * height * 4],
            width,
            height,
        }
    }

    /// Blend `src` onto the canvas at `(dx, dy)`, clipping to the canvas.
    ///
    /// Opaque sources are copied row by row; translucent pixels use the same
    /// source-over formula the compositor always used (`dst = (src*a + dst*(1-a))`,
    /// result alpha forced to 255) so output stays bit-identical to the previous
    /// per-pixel implementation.
    pub fn blit(&mut self, dx: i32, dy: i32, src: &SurfacePixels) {
        if src.width == 0 || src.height == 0 || self.width == 0 || self.height == 0 {
            return;
        }
        let x0 = dx.max(0);
        let y0 = dy.max(0);
        let x1 = (dx + src.width as i32).min(self.width as i32);
        let y1 = (dy + src.height as i32).min(self.height as i32);
        if x1 <= x0 || y1 <= y0 {
            return;
        }
        let (sx, sy) = ((x0 - dx) as usize, (y0 - dy) as usize);
        let span = (x1 - x0) as usize;
        let span_bytes = span * 4;
        for row in 0..(y1 - y0) as usize {
            let src_start = ((sy + row) * src.width + sx) * 4;
            let src_row = &src.rgba[src_start..src_start + span_bytes];
            let dst_start = ((y0 as usize + row) * self.width + x0 as usize) * 4;
            let dst_row = &mut self.pixels[dst_start..dst_start + span_bytes];
            if src.opaque {
                dst_row.copy_from_slice(src_row);
                continue;
            }
            // ARGB8888 surfaces are usually fully opaque in practice (terminals,
            // editors, browsers). Scanning the alpha bytes is far cheaper than
            // three divisions per pixel, and lets those rows take the memcpy path.
            if (3..span_bytes)
                .step_by(4)
                .all(|offset| src_row[offset] == 255)
            {
                dst_row.copy_from_slice(src_row);
                continue;
            }
            for offset in (0..span_bytes).step_by(4) {
                let alpha = src_row[offset + 3] as u32;
                if alpha == 0 {
                    continue;
                }
                if alpha == 255 {
                    dst_row[offset..offset + 4].copy_from_slice(&src_row[offset..offset + 4]);
                    continue;
                }
                let inv = 255 - alpha;
                for channel in 0..3 {
                    dst_row[offset + channel] = ((src_row[offset + channel] as u32 * alpha
                        + dst_row[offset + channel] as u32 * inv)
                        / 255) as u8;
                }
                dst_row[offset + 3] = 255;
            }
        }
    }

    pub fn into_pixels(self) -> Vec<u8> {
        self.pixels
    }
}

/// Bounding box of the pixels that differ, in RGBA8888 top-left origin.
///
/// Whole rows are compared with `memcmp` first; only rows that changed are
/// scanned pixel by pixel, which is what makes this cheap enough to run on
/// every commit.
pub fn diff_bbox(
    previous: &[u8],
    current: &[u8],
    width: usize,
    height: usize,
) -> Option<(usize, usize, usize, usize)> {
    let stride = width * 4;
    if width == 0 || height == 0 {
        return None;
    }
    if previous.len() < stride * height || current.len() < stride * height {
        return None;
    }
    let (mut min_x, mut min_y, mut max_x, mut max_y) = (width, height, 0usize, 0usize);
    let mut found = false;
    for y in 0..height {
        let a = &previous[y * stride..(y + 1) * stride];
        let b = &current[y * stride..(y + 1) * stride];
        if a == b {
            continue;
        }
        let first = (0..width)
            .find(|x| a[x * 4..x * 4 + 4] != b[x * 4..x * 4 + 4])
            .unwrap_or(0);
        let last = (0..width)
            .rev()
            .find(|x| a[x * 4..x * 4 + 4] != b[x * 4..x * 4 + 4])
            .unwrap_or(0);
        found = true;
        min_x = min_x.min(first);
        max_x = max_x.max(last);
        min_y = min_y.min(y);
        max_y = max_y.max(y);
    }
    if !found {
        return None;
    }
    Some((min_x, min_y, max_x - min_x + 1, max_y - min_y + 1))
}

/// Copy the `(x, y, w, h)` rectangle out of a tightly packed RGBA8888 frame.
pub fn extract(rgba: &[u8], width: usize, x: usize, y: usize, w: usize, h: usize) -> Vec<u8> {
    let mut patch = vec![0u8; w * h * 4];
    for row in 0..h {
        let start = ((y + row) * width + x) * 4;
        patch[row * w * 4..(row + 1) * w * 4].copy_from_slice(&rgba[start..start + w * 4]);
    }
    patch
}

/// Which trade-off a frame should be encoded with.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PngMode {
    /// Damage patch: latency dominates, the rect is usually small and cheap to
    /// re-encode once per commit.
    Patch,
    /// Full frame (attach, resize): encoded rarely, so keep the bytes small.
    Full,
}

/// PNG-encode RGBA8888 pixels.
pub fn encode_png(rgba: &[u8], width: usize, height: usize, mode: PngMode) -> Option<Vec<u8>> {
    if width == 0 || height == 0 {
        return None;
    }
    let mut out = Vec::new();
    {
        let mut encoder = Encoder::new(&mut out, width as u32, height as u32);
        encoder.set_color(ColorType::Rgba);
        encoder.set_depth(BitDepth::Eight);
        match mode {
            // No per-row filter search and the cheapest deflate level: several
            // times faster than the default, for frames that are already
            // dominated by transport.
            PngMode::Patch => {
                encoder.set_compression(Compression::Fast);
                encoder.set_filter(FilterType::NoFilter);
            }
            PngMode::Full => {
                encoder.set_compression(Compression::Fast);
                encoder.set_filter(FilterType::Sub);
            }
        }
        let mut writer = encoder.write_header().ok()?;
        writer.write_image_data(rgba).ok()?;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Deterministic xorshift so the tests do not need a rand dependency.
    fn rng(seed: &mut u64) -> u32 {
        *seed ^= *seed << 13;
        *seed ^= *seed >> 7;
        *seed ^= *seed << 17;
        (*seed >> 32) as u32
    }

    #[allow(clippy::too_many_arguments)]
    fn reference_blit(
        canvas: &mut [u8],
        canvas_w: usize,
        canvas_h: usize,
        dx: i32,
        dy: i32,
        src: &[u8],
        src_w: usize,
        src_h: usize,
    ) {
        for row in 0..src_h {
            let y = dy + row as i32;
            if y < 0 || y >= canvas_h as i32 {
                continue;
            }
            for col in 0..src_w {
                let x = dx + col as i32;
                if x < 0 || x >= canvas_w as i32 {
                    continue;
                }
                let s = (row * src_w + col) * 4;
                let d = (y as usize * canvas_w + x as usize) * 4;
                let alpha = src[s + 3] as u32;
                if alpha == 0 {
                    continue;
                }
                if alpha == 255 {
                    canvas[d..d + 4].copy_from_slice(&src[s..s + 4]);
                } else {
                    let inv = 255 - alpha;
                    for channel in 0..3 {
                        canvas[d + channel] = ((src[s + channel] as u32 * alpha
                            + canvas[d + channel] as u32 * inv)
                            / 255) as u8;
                    }
                    canvas[d + 3] = 255;
                }
            }
        }
    }

    fn reference_diff(
        previous: &[u8],
        current: &[u8],
        width: usize,
        height: usize,
    ) -> Option<(usize, usize, usize, usize)> {
        let mut min_x = width;
        let mut min_y = height;
        let mut max_x = 0usize;
        let mut max_y = 0usize;
        let mut found = false;
        for y in 0..height {
            let a = &previous[y * width * 4..(y + 1) * width * 4];
            let b = &current[y * width * 4..(y + 1) * width * 4];
            if a == b {
                continue;
            }
            let mut first = None;
            let mut last = 0usize;
            for x in 0..width {
                if a[x * 4..x * 4 + 4] != b[x * 4..x * 4 + 4] {
                    first.get_or_insert(x);
                    last = x;
                }
            }
            if let Some(first) = first {
                found = true;
                min_x = min_x.min(first);
                max_x = max_x.max(last);
                min_y = min_y.min(y);
                max_y = max_y.max(y);
            }
        }
        found.then_some((min_x, min_y, max_x - min_x + 1, max_y - min_y + 1))
    }

    fn reference_bgra(
        source: &[u8],
        stride: usize,
        width: usize,
        height: usize,
        opaque: bool,
    ) -> Vec<u8> {
        let mut rgba = vec![0u8; width * height * 4];
        for row in 0..height {
            let src = &source[row * stride..row * stride + width * 4];
            let dst = &mut rgba[row * width * 4..(row + 1) * width * 4];
            for col in 0..width {
                let b = src[col * 4];
                let g = src[col * 4 + 1];
                let r = src[col * 4 + 2];
                let a = if opaque { 255 } else { src[col * 4 + 3] };
                dst[col * 4] = r;
                dst[col * 4 + 1] = g;
                dst[col * 4 + 2] = b;
                dst[col * 4 + 3] = a;
            }
        }
        rgba
    }

    #[test]
    fn bgra_matches_reference() {
        for &(w, h, stride_pad) in &[(1usize, 1usize, 0usize), (7, 5, 13), (64, 33, 0)] {
            let stride = w * 4 + stride_pad;
            for &opaque in &[false, true] {
                let mut seed = 0x1234_5678_9abc_def0u64;
                let source: Vec<u8> = (0..stride * h).map(|_| rng(&mut seed) as u8).collect();
                let actual = bgra_to_rgba(&source, stride, w, h, opaque);
                assert_eq!((actual.width, actual.height, actual.opaque), (w, h, opaque));
                assert_eq!(
                    actual.rgba,
                    reference_bgra(&source, stride, w, h, opaque),
                    "w={w} h={h} stride={stride} opaque={opaque}"
                );
            }
        }
    }

    #[test]
    fn blit_matches_reference_for_clipped_rects() {
        let (canvas_w, canvas_h) = (37usize, 23usize);
        let mut seed = 0xdead_beef_cafe_f00du64;
        let base: Vec<u8> = (0..canvas_w * canvas_h * 4)
            .map(|_| rng(&mut seed) as u8)
            .collect();
        for &(src_w, src_h) in &[(1usize, 1usize), (5, 3), (40, 30)] {
            for &(dx, dy) in &[
                (0, 0),
                (-3, -2),
                (5, 4),
                (canvas_w as i32 - 2, canvas_h as i32 - 2),
                (-(src_w as i32), 3),
            ] {
                for &opaque in &[false, true] {
                    let mut rgba: Vec<u8> = (0..src_w * src_h * 4)
                        .map(|_| rng(&mut seed) as u8)
                        .collect();
                    if opaque {
                        for offset in (3..rgba.len()).step_by(4) {
                            rgba[offset] = 255;
                        }
                    }
                    let mut expected = base.clone();
                    reference_blit(
                        &mut expected,
                        canvas_w,
                        canvas_h,
                        dx,
                        dy,
                        &rgba,
                        src_w,
                        src_h,
                    );

                    let mut canvas = Canvas::new(canvas_w, canvas_h);
                    canvas.pixels.copy_from_slice(&base);
                    canvas.blit(
                        dx,
                        dy,
                        &SurfacePixels {
                            width: src_w,
                            height: src_h,
                            rgba,
                            opaque,
                        },
                    );
                    assert_eq!(
                        canvas.into_pixels(),
                        expected,
                        "src={src_w}x{src_h} at {dx},{dy} opaque={opaque}"
                    );
                }
            }
        }
    }

    #[test]
    fn diff_bbox_matches_reference() {
        let (w, h) = (41usize, 17usize);
        let mut seed = 0x0f0f_1234_5678_9abcu64;
        let original: Vec<u8> = (0..w * h * 4).map(|_| rng(&mut seed) as u8).collect();
        let cases = [
            (0, 0, 1, 1),
            (3, 2, 10, 4),
            (w - 1, h - 1, 1, 1),
            (7, 0, 1, h),
        ];
        for (x, y, rw, rh) in cases {
            let mut changed = original.clone();
            for row in y..y + rh {
                for col in x..x + rw {
                    let index = (row * w + col) * 4;
                    changed[index] ^= 0xff;
                }
            }
            assert_eq!(
                diff_bbox(&original, &changed, w, h),
                reference_diff(&original, &changed, w, h),
                "rect {x},{y} {rw}x{rh}"
            );
        }
        assert_eq!(diff_bbox(&original, &original, w, h), None);
    }

    #[test]
    fn extract_round_trips_into_canvas() {
        let (w, h) = (9usize, 6usize);
        let mut seed = 0x1111_2222_3333_4444u64;
        let frame: Vec<u8> = (0..w * h * 4).map(|_| rng(&mut seed) as u8).collect();
        let patch = extract(&frame, w, 2, 1, 4, 3);
        let mut canvas = Canvas::new(w, h);
        canvas.blit(
            2,
            1,
            &SurfacePixels {
                width: 4,
                height: 3,
                rgba: patch,
                opaque: true,
            },
        );
        let pixels = canvas.into_pixels();
        for row in 1..4 {
            for col in 2..6 {
                let index = (row * w + col) * 4;
                assert_eq!(&pixels[index..index + 4], &frame[index..index + 4]);
            }
        }
    }

    #[test]
    fn png_round_trips_pixels() {
        let (w, h) = (8usize, 5usize);
        let mut seed = 0x9999_8888_7777_6666u64;
        let frame: Vec<u8> = (0..w * h * 4).map(|_| rng(&mut seed) as u8).collect();
        for mode in [PngMode::Patch, PngMode::Full] {
            let encoded = encode_png(&frame, w, h, mode).expect("encode");
            let mut decoder = png::Decoder::new(encoded.as_slice());
            decoder.set_transformations(png::Transformations::EXPAND);
            let mut reader = decoder.read_info().expect("header");
            let mut buf = vec![0u8; reader.output_buffer_size()];
            let info = reader.next_frame(&mut buf).expect("frame");
            assert_eq!((info.width, info.height), (w as u32, h as u32));
            assert_eq!(&buf[..info.buffer_size()], &frame[..]);
        }
    }

    /// Not a pass/fail test: prints timings for the paths we changed.
    /// Run with `cargo test -p mixum-env-compositor --release -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn bench_hot_paths() {
        use std::time::Instant;
        let (w, h) = (1280usize, 800usize);
        let mut seed = 0xabcd_ef01_2345_6789u64;
        let frame: Vec<u8> = (0..w * h * 4).map(|_| rng(&mut seed) as u8).collect();
        let mut other = frame.clone();
        for y in 300..380 {
            for x in 100..900 {
                let i = (y * w + x) * 4;
                other[i] ^= 0x5a;
            }
        }

        let rounds = 20;
        let t = Instant::now();
        for _ in 0..rounds {
            std::hint::black_box(diff_bbox(&frame, &other, w, h));
        }
        println!("diff_bbox         {:?}/frame", t.elapsed() / rounds);

        let t = Instant::now();
        for _ in 0..rounds {
            std::hint::black_box(reference_diff(&frame, &other, w, h));
        }
        println!("diff_bbox (old)   {:?}/frame", t.elapsed() / rounds);

        let mut canvas = Canvas::new(w, h);
        let blended = SurfacePixels {
            width: w,
            height: h,
            rgba: frame.clone(),
            opaque: false,
        };
        let t = Instant::now();
        for _ in 0..rounds {
            canvas.blit(0, 0, &blended);
        }
        println!("blit_blend        {:?}/frame", t.elapsed() / rounds);

        // ARGB surface whose pixels happen to be opaque (the common case).
        let mut opaque_content = frame.clone();
        for offset in (3..opaque_content.len()).step_by(4) {
            opaque_content[offset] = 255;
        }
        let argb_opaque = SurfacePixels {
            width: w,
            height: h,
            rgba: opaque_content,
            opaque: false,
        };
        let t = Instant::now();
        for _ in 0..rounds {
            canvas.blit(0, 0, &argb_opaque);
        }
        println!("blit_argb_opaque  {:?}/frame", t.elapsed() / rounds);

        let xrgb = SurfacePixels {
            width: w,
            height: h,
            rgba: frame.clone(),
            opaque: true,
        };
        let t = Instant::now();
        for _ in 0..rounds {
            canvas.blit(0, 0, &xrgb);
        }
        println!("blit_opaque       {:?}/frame", t.elapsed() / rounds);

        let t = Instant::now();
        for _ in 0..rounds {
            std::hint::black_box(bgra_to_rgba(&frame, w * 4, w, h, false));
        }
        println!("bgra_to_rgba      {:?}/frame", t.elapsed() / rounds);

        let patch = extract(&other, w, 100, 300, 800, 80);
        let t = Instant::now();
        for _ in 0..rounds {
            std::hint::black_box(encode_png(&patch, 800, 80, PngMode::Patch));
        }
        println!("png patch (fast)  {:?}/frame", t.elapsed() / rounds);

        let t = Instant::now();
        for _ in 0..rounds {
            std::hint::black_box(encode_png(&patch, 800, 80, PngMode::Full));
        }
        println!("png patch (full)  {:?}/frame", t.elapsed() / rounds);

        let t = Instant::now();
        for _ in 0..rounds {
            std::hint::black_box(encode_png(&frame, w, h, PngMode::Patch));
        }
        println!("png full (fast)   {:?}/frame", t.elapsed() / rounds);

        let t = Instant::now();
        for _ in 0..rounds {
            std::hint::black_box(encode_png(&frame, w, h, PngMode::Full));
        }
        println!("png full (full)   {:?}/frame", t.elapsed() / rounds);
    }
}
