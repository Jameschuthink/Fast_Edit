//! CSS colors as Canvas2D accepts them: #rgb #rgba #rrggbb #rrggbbaa, rgb()/rgba() (commas or spaces, % allowed),
//! hsl()/hsla(), the named colors, `transparent` and `currentcolor` (black). Unknown strings return None (the
//! browser ignores them, so the caller keeps the previous style).

use skia_safe::Color4f;

pub fn parse(s: &str) -> Option<Color4f> {
    let s = s.trim();
    if let Some(hex) = s.strip_prefix('#') {
        return parse_hex(hex);
    }
    let lower = s.to_ascii_lowercase();
    if let Some(open) = lower.find('(') {
        let name = lower[..open].trim();
        let inner = lower[open + 1..].trim_end().strip_suffix(')')?;
        let parts: Vec<&str> = inner
            .split(|c| c == ',' || c == ' ' || c == '/')
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .collect();
        return match name {
            "rgb" | "rgba" => parse_rgb(&parts),
            "hsl" | "hsla" => parse_hsl(&parts),
            _ => None,
        };
    }
    named(&lower)
}

fn hexval(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

fn parse_hex(h: &str) -> Option<Color4f> {
    let b = h.as_bytes();
    let v: Option<Vec<u8>> = b.iter().map(|&c| hexval(c)).collect();
    let v = v?;
    let (r, g, bl, a) = match v.len() {
        3 => (v[0] * 17, v[1] * 17, v[2] * 17, 255),
        4 => (v[0] * 17, v[1] * 17, v[2] * 17, v[3] * 17),
        6 => (v[0] * 16 + v[1], v[2] * 16 + v[3], v[4] * 16 + v[5], 255),
        8 => (v[0] * 16 + v[1], v[2] * 16 + v[3], v[4] * 16 + v[5], v[6] * 16 + v[7]),
        _ => return None,
    };
    Some(Color4f::new(r as f32 / 255., g as f32 / 255., bl as f32 / 255., a as f32 / 255.))
}

fn num(p: &str) -> Option<f32> {
    p.parse::<f32>().ok().filter(|v| v.is_finite())
}

fn channel(p: &str) -> Option<f32> {
    if let Some(pc) = p.strip_suffix('%') {
        Some((num(pc)? / 100.).clamp(0., 1.))
    } else {
        Some((num(p)? / 255.).clamp(0., 1.))
    }
}

fn alpha(p: Option<&&str>) -> Option<f32> {
    match p {
        None => Some(1.),
        Some(p) => {
            if let Some(pc) = p.strip_suffix('%') {
                Some((num(pc)? / 100.).clamp(0., 1.))
            } else {
                Some(num(p)?.clamp(0., 1.))
            }
        }
    }
}

fn parse_rgb(p: &[&str]) -> Option<Color4f> {
    if p.len() < 3 {
        return None;
    }
    Some(Color4f::new(channel(p[0])?, channel(p[1])?, channel(p[2])?, alpha(p.get(3))?))
}

fn parse_hsl(p: &[&str]) -> Option<Color4f> {
    if p.len() < 3 {
        return None;
    }
    let h = p[0].trim_end_matches("deg");
    let h = (num(h)? / 360.).rem_euclid(1.);
    let s = num(p[1].trim_end_matches('%'))? / 100.;
    let l = num(p[2].trim_end_matches('%'))? / 100.;
    let a = alpha(p.get(3))?;
    let f = |n: f32| {
        let k = (n + h * 12.) % 12.;
        let aa = s.clamp(0., 1.) * l.min(1. - l);
        l - aa * (-1f32).max((k - 3.).min(9. - k).min(1.))
    };
    Some(Color4f::new(f(0.).clamp(0., 1.), f(8.).clamp(0., 1.), f(4.).clamp(0., 1.), a))
}

fn named(n: &str) -> Option<Color4f> {
    let rgb: u32 = match n {
        "transparent" => return Some(Color4f::new(0., 0., 0., 0.)),
        "currentcolor" => 0x000000,
        "black" => 0x000000, "white" => 0xffffff, "red" => 0xff0000, "green" => 0x008000, "blue" => 0x0000ff,
        "yellow" => 0xffff00, "cyan" | "aqua" => 0x00ffff, "magenta" | "fuchsia" => 0xff00ff, "gray" | "grey" => 0x808080,
        "silver" => 0xc0c0c0, "maroon" => 0x800000, "olive" => 0x808000, "lime" => 0x00ff00, "teal" => 0x008080,
        "navy" => 0x000080, "purple" => 0x800080, "orange" => 0xffa500, "pink" => 0xffc0cb, "gold" => 0xffd700,
        "brown" => 0xa52a2a, "coral" => 0xff7f50, "crimson" => 0xdc143c, "darkgray" | "darkgrey" => 0xa9a9a9,
        "dimgray" | "dimgrey" => 0x696969, "lightgray" | "lightgrey" => 0xd3d3d3, "whitesmoke" => 0xf5f5f5,
        "indigo" => 0x4b0082, "violet" => 0xee82ee, "orchid" => 0xda70d6, "salmon" => 0xfa8072, "tomato" => 0xff6347,
        "skyblue" => 0x87ceeb, "steelblue" => 0x4682b4, "royalblue" => 0x4169e1, "dodgerblue" => 0x1e90ff,
        "deepskyblue" => 0x00bfff, "turquoise" => 0x40e0d0, "seagreen" => 0x2e8b57, "limegreen" => 0x32cd32,
        "forestgreen" => 0x228b22, "chartreuse" => 0x7fff00, "khaki" => 0xf0e68c, "beige" => 0xf5f5dc, "ivory" => 0xfffff0,
        "lavender" => 0xe6e6fa, "plum" => 0xdda0dd, "tan" => 0xd2b48c, "chocolate" => 0xd2691e, "firebrick" => 0xb22222,
        "darkred" => 0x8b0000, "darkblue" => 0x00008b, "darkgreen" => 0x006400, "slategray" | "slategrey" => 0x708090,
        "hotpink" => 0xff69b4, "deeppink" => 0xff1493, "orangered" => 0xff4500, "darkorange" => 0xff8c00,
        "goldenrod" => 0xdaa520, "mintcream" => 0xf5fffa, "snow" => 0xfffafa, "linen" => 0xfaf0e6,
        _ => return None,
    };
    Some(Color4f::new(
        ((rgb >> 16) & 255) as f32 / 255.,
        ((rgb >> 8) & 255) as f32 / 255.,
        (rgb & 255) as f32 / 255.,
        1.,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn colors() {
        let c = parse("rgba(239,236,230,0.22)").unwrap();
        assert!((c.r - 239. / 255.).abs() < 1e-6 && (c.a - 0.22).abs() < 1e-6);
        assert_eq!(parse("#fff").unwrap().r, 1.0);
        assert!((parse("#7c6cff").unwrap().b - 1.0).abs() < 1e-6);
        assert!(parse("hsl(120, 100%, 50%)").unwrap().g > 0.99);
        assert!(parse("rgb(10 20 30 / 50%)").unwrap().a - 0.5 < 1e-6);
        assert!(parse("nonsense").is_none());
    }
}
