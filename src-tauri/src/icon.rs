/// Generate the Pointeuse tray icon as raw RGBA pixels.
/// Design: Blue circle arc (clock) with white "P" letterform, transparent background.
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
    // White for H letterform — full brightness
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

            // "P" letterform — stem + bowl, bold and clear
            let p_top = center - s * 0.22;
            let p_bottom = center + s * 0.22;
            let stem_x = center - s * 0.11;
            let bar_width = s * 0.10;
            let bowl_mid_r = s * 0.09;
            let bowl_cy = p_top + bowl_mid_r + bar_width / 2.0;

            // Stem (left vertical, full letter height)
            if fx >= stem_x - bar_width / 2.0 && fx <= stem_x + bar_width / 2.0
                && fy >= p_top && fy <= p_bottom
            {
                let aa_x = soft_edge(fx, stem_x - bar_width / 2.0, stem_x + bar_width / 2.0);
                let aa_y = soft_edge(fy, p_top, p_bottom);
                blend_pixel(&mut pixels[idx..idx + 4], &white, aa_x * aa_y);
            }

            // Bowl (right half-annulus attached to the stem's upper part)
            let bdx = fx - stem_x;
            let bdy = fy - bowl_cy;
            let bdist = (bdx * bdx + bdy * bdy).sqrt();
            let bowl_inner = bowl_mid_r - bar_width / 2.0;
            let bowl_outer = bowl_mid_r + bar_width / 2.0;
            if bdx >= 0.0 && bdist >= bowl_inner - 1.0 && bdist <= bowl_outer + 1.0 {
                let aa = antialiased_ring(bdist, bowl_inner, bowl_outer);
                if aa > 0.0 {
                    blend_pixel(&mut pixels[idx..idx + 4], &white, aa);
                }
            }
        }
    }

    (size, size, pixels)
}

fn antialiased_ring(dist: f64, inner: f64, outer: f64) -> f64 {
    let aa = 1.0;
    let inner_aa = ((dist - inner) / aa + 0.5).clamp(0.0, 1.0);
    let outer_aa = ((outer - dist) / aa + 0.5).clamp(0.0, 1.0);
    inner_aa * outer_aa
}

fn soft_edge(val: f64, min: f64, max: f64) -> f64 {
    let aa = 0.8;
    let lo = ((val - min) / aa + 0.5).clamp(0.0, 1.0);
    let hi = ((max - val) / aa + 0.5).clamp(0.0, 1.0);
    lo * hi
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
