/// A small clock glyph used for the window and the tray.
pub fn clock_icon_rgba(size: u32) -> Vec<u8> {
    let mut rgba = vec![0_u8; (size * size * 4) as usize];
    let center = (size as f32 - 1.0) / 2.0;
    let radius = center * 0.86;
    for y in 0..size {
        for x in 0..size {
            let dx = x as f32 - center;
            let dy = y as f32 - center;
            let distance = (dx * dx + dy * dy).sqrt();
            if distance <= radius {
                put(&mut rgba, size, x, y, [24, 78, 130]);
            }
        }
    }
    let hour = point_at(center, radius * 0.42, -50.0);
    let minute = point_at(center, radius * 0.68, 35.0);
    stamp_line(
        &mut rgba,
        size,
        center,
        center,
        hour.0,
        hour.1,
        [245, 247, 250],
    );
    stamp_line(
        &mut rgba,
        size,
        center,
        center,
        minute.0,
        minute.1,
        [245, 247, 250],
    );
    put(
        &mut rgba,
        size,
        center as u32,
        center as u32,
        [245, 247, 250],
    );
    rgba
}

fn point_at(center: f32, length: f32, degrees_from_12: f32) -> (f32, f32) {
    let radians = degrees_from_12.to_radians();
    (
        center + length * radians.sin(),
        center - length * radians.cos(),
    )
}

fn stamp_line(rgba: &mut [u8], size: u32, x0: f32, y0: f32, x1: f32, y1: f32, color: [u8; 3]) {
    let steps = size * 2;
    for step in 0..=steps {
        let t = step as f32 / steps as f32;
        let x = x0 + (x1 - x0) * t;
        let y = y0 + (y1 - y0) * t;
        for dy in -1..=1 {
            for dx in -1..=1 {
                let px = x.round() as i32 + dx;
                let py = y.round() as i32 + dy;
                if px >= 0 && py >= 0 && (px as u32) < size && (py as u32) < size {
                    put(rgba, size, px as u32, py as u32, color);
                }
            }
        }
    }
}

fn put(rgba: &mut [u8], size: u32, x: u32, y: u32, color: [u8; 3]) {
    if x >= size || y >= size {
        return;
    }
    let index = ((y * size + x) * 4) as usize;
    rgba[index] = color[0];
    rgba[index + 1] = color[1];
    rgba[index + 2] = color[2];
    rgba[index + 3] = 255;
}
