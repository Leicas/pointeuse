/// Generates app-icon.png from the procedural icon in icon.rs
/// Run: cargo run --bin gen_icon
use std::fs::File;
use std::io::BufWriter;
use std::path::Path;

// Inline the icon generation (same logic as icon.rs)
fn generate_icon(size: u32) -> (u32, u32, Vec<u8>) {
    let s = size as f64;
    let center = s / 2.0;
    let radius = s * 0.42;
    let ring_width = s * 0.09;
    let mut pixels = vec![0u8; (size * size * 4) as usize];

    let brand_bright = [96u8, 165, 250, 255];
    let brand_dim = [59u8, 130, 246, 255];
    let white = [255u8, 255, 255, 255];

    for y in 0..size {
        for x in 0..size {
            let fx = x as f64 + 0.5;
            let fy = y as f64 + 0.5;
            let dx = fx - center;
            let dy = fy - center;
            let dist = (dx * dx + dy * dy).sqrt();
            let idx = ((y * size + x) * 4) as usize;

            let ring_inner = radius - ring_width / 2.0;
            let ring_outer = radius + ring_width / 2.0;
            if dist >= ring_inner - 1.0 && dist <= ring_outer + 1.0 {
                let aa = antialiased_ring(dist, ring_inner, ring_outer);
                if aa > 0.0 {
                    let angle = dy.atan2(dx);
                    let is_bright_arc =
                        (-std::f64::consts::FRAC_PI_2..=0.0).contains(&angle);
                    let (color, alpha) = if is_bright_arc {
                        (&brand_bright, aa)
                    } else {
                        (&brand_dim, aa * 0.35)
                    };
                    blend_pixel(&mut pixels[idx..idx + 4], color, alpha);
                }
            }

            // Checkmark — two rounded strokes meeting at the vertex, reading
            // as the clock's hands. Drawn via distance-to-segment (capsules).
            let vx = center - s * 0.04; // vertex (lowest point of the check)
            let vy = center + s * 0.14;
            let short_x = center - s * 0.20; // upper-left (short hand)
            let short_y = center - s * 0.02;
            let long_x = center + s * 0.22; // upper-right (long hand)
            let long_y = center - s * 0.18;
            let stroke_hw = s * 0.055; // half stroke width

            let d_short = dist_to_segment(fx, fy, vx, vy, short_x, short_y);
            let d_long = dist_to_segment(fx, fy, vx, vy, long_x, long_y);
            let check_aa = (stroke_hw + 0.75 - d_short.min(d_long)).clamp(0.0, 1.0);
            if check_aa > 0.0 {
                blend_pixel(&mut pixels[idx..idx + 4], &white, check_aa);
            }
        }
    }
    (size, size, pixels)
}

/// Shortest distance from point (px,py) to the segment (ax,ay)-(bx,by).
fn dist_to_segment(px: f64, py: f64, ax: f64, ay: f64, bx: f64, by: f64) -> f64 {
    let abx = bx - ax;
    let aby = by - ay;
    let ab2 = abx * abx + aby * aby;
    let t = if ab2 == 0.0 {
        0.0
    } else {
        (((px - ax) * abx + (py - ay) * aby) / ab2).clamp(0.0, 1.0)
    };
    let dx = px - (ax + t * abx);
    let dy = py - (ay + t * aby);
    (dx * dx + dy * dy).sqrt()
}

fn antialiased_ring(dist: f64, inner: f64, outer: f64) -> f64 {
    let aa = 1.0;
    let inner_aa = ((dist - inner) / aa + 0.5).clamp(0.0, 1.0);
    let outer_aa = ((outer - dist) / aa + 0.5).clamp(0.0, 1.0);
    inner_aa * outer_aa
}

fn blend_pixel(dst: &mut [u8], color: &[u8; 4], alpha: f64) {
    let a = (alpha * color[3] as f64) as u16;
    if a == 0 {
        return;
    }
    let inv_a = 255 - a;
    dst[0] = ((color[0] as u16 * a + dst[0] as u16 * inv_a) / 255) as u8;
    dst[1] = ((color[1] as u16 * a + dst[1] as u16 * inv_a) / 255) as u8;
    dst[2] = ((color[2] as u16 * a + dst[2] as u16 * inv_a) / 255) as u8;
    dst[3] = (dst[3] as u16 + a - (dst[3] as u16 * a / 255)).min(255) as u8;
}

fn write_png(path: &Path, width: u32, height: u32, rgba: &[u8]) {
    let file = File::create(path).expect("Failed to create file");
    let w = BufWriter::new(file);
    let mut encoder = png::Encoder::new(w, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().expect("Failed to write PNG header");
    writer.write_image_data(rgba).expect("Failed to write PNG data");
}

fn main() {
    let size: u32 = 1024;
    let (w, h, rgba) = generate_icon(size);

    // Add a solid dark rounded-rect background behind the icon
    let bg = [15u8, 18, 25, 255]; // #0f1219 — matches app background
    let corner_radius = size as f64 * 0.18; // rounded corners

    // Work backwards: composite icon over background
    let mut final_pixels = vec![0u8; (size * size * 4) as usize];
    for y in 0..size {
        for x in 0..size {
            let idx = ((y * size + x) * 4) as usize;
            let fx = x as f64 + 0.5;
            let fy = y as f64 + 0.5;

            // Rounded rect mask
            let r = corner_radius;
            let in_x = fx.clamp(r, size as f64 - r);
            let in_y = fy.clamp(r, size as f64 - r);
            let dx = fx - in_x;
            let dy = fy - in_y;
            let dist = (dx * dx + dy * dy).sqrt();
            let mask = ((r - dist) + 0.5).clamp(0.0, 1.0);

            if mask > 0.0 {
                // Background
                let bg_a = (mask * 255.0) as u8;
                final_pixels[idx] = bg[0];
                final_pixels[idx + 1] = bg[1];
                final_pixels[idx + 2] = bg[2];
                final_pixels[idx + 3] = bg_a;

                // Composite icon on top
                let src_a = rgba[idx + 3] as f64 / 255.0 * mask;
                if src_a > 0.0 {
                    let inv = 1.0 - src_a;
                    final_pixels[idx] =
                        (rgba[idx] as f64 * src_a + final_pixels[idx] as f64 * inv) as u8;
                    final_pixels[idx + 1] =
                        (rgba[idx + 1] as f64 * src_a + final_pixels[idx + 1] as f64 * inv) as u8;
                    final_pixels[idx + 2] =
                        (rgba[idx + 2] as f64 * src_a + final_pixels[idx + 2] as f64 * inv) as u8;
                    final_pixels[idx + 3] =
                        (final_pixels[idx + 3] as f64 + (255.0 - final_pixels[idx + 3] as f64) * src_a)
                            .min(255.0) as u8;
                }
            }
        }
    }

    let out = Path::new("app-icon.png");
    write_png(out, w, h, &final_pixels);
    println!("Generated {} ({}x{})", out.display(), w, h);
}
