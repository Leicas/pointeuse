/// Generate the Pointeuse tray icon as raw RGBA pixels.
/// Design: blue clock ring (bright top-right arc) with a white checkmark whose
/// two strokes read as the clock's hands — "time, tracked". Transparent bg.
/// Rendered at 64x64 for HiDPI crispness — Windows will downscale as needed.
/// Returns (width, height, rgba_bytes).
pub fn generate_tray_icon(size: u32) -> (u32, u32, Vec<u8>) {
    let s = size as f64;
    let center = s / 2.0;
    let radius = s * 0.42;
    let ring_width = s * 0.09;
    let mut pixels = vec![0u8; (size * size * 4) as usize];

    // Brand bright: #60a5fa (full brightness for active arc)
    let brand_bright = [96u8, 165, 250, 255];
    // Brand blue: #3b82f6 at 35% opacity for the rest of the ring
    let brand_dim = [59u8, 130, 246, 255];
    // White for the checkmark — full brightness, crisp at small sizes
    let white = [255u8, 255, 255, 255];

    for y in 0..size {
        for x in 0..size {
            let fx = x as f64 + 0.5;
            let fy = y as f64 + 0.5;
            let dx = fx - center;
            let dy = fy - center;
            let dist = (dx * dx + dy * dy).sqrt();
            let idx = ((y * size + x) * 4) as usize;

            // Circle ring
            let ring_inner = radius - ring_width / 2.0;
            let ring_outer = radius + ring_width / 2.0;
            if dist >= ring_inner - 1.0 && dist <= ring_outer + 1.0 {
                let aa = antialiased_ring(dist, ring_inner, ring_outer);
                if aa > 0.0 {
                    // Top-right quarter arc: full bright brand blue
                    // Rest of ring: dimmed at 35% opacity
                    let angle = dy.atan2(dx);
                    let is_bright_arc = (-std::f64::consts::FRAC_PI_2..=0.0).contains(&angle);
                    let (color, alpha) = if is_bright_arc {
                        (&brand_bright, aa)
                    } else {
                        (&brand_dim, aa * 0.35)
                    };
                    blend_pixel(&mut pixels[idx..idx + 4], color, alpha);
                }
            }

            // Checkmark — two rounded strokes meeting at the vertex, reading
            // as the clock's hands. Drawn via distance-to-segment (capsules),
            // which gives clean rounded caps and crisp antialiasing.
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
    if a == 0 { return; }
    let inv_a = 255 - a;
    dst[0] = ((color[0] as u16 * a + dst[0] as u16 * inv_a) / 255) as u8;
    dst[1] = ((color[1] as u16 * a + dst[1] as u16 * inv_a) / 255) as u8;
    dst[2] = ((color[2] as u16 * a + dst[2] as u16 * inv_a) / 255) as u8;
    dst[3] = (dst[3] as u16 + a - (dst[3] as u16 * a / 255)).min(255) as u8;
}
