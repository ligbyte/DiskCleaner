//! 极简 SVG 光栅化：只为了把 `assets/` 里的品牌图标（qq.svg / wechat.svg）画到界面上。
//!
//! 为什么不引 `resvg`：这两个图标只有 `<path d>` 一个元素，没有 transform、渐变、文字，
//! 为了两个图形拖进 usvg + tiny-skia 一整套依赖并不划算；而且项目坚持**单文件分发**，
//! 源 SVG 用 `include_bytes!` 编进二进制，换图标只要替换 assets 里的文件即可。
//!
//! 支持范围（够用即止，不追求规范完备）：
//!
//! * 路径命令 `M/m L/l H/h V/v C/c S/s Q/q T/t A/a Z/z`，含隐式重复与相对坐标
//! * 多子路径、`nonzero` / `evenodd` 两种填充规则
//! * `fill` 的 `#RGB` / `#RRGGBB` / `#RRGGBBAA` 与 `none`
//! * `viewBox` 等比缩放到目标像素
//!
//! 其它元素（`circle` / `rect` / `<use>` / 渐变 …）直接忽略 —— 现在的图标用不到，
//! 真需要了再加，别为了"完整"把复杂度堆上去。
//!
//! 光栅化本身是**扫描线 + 4×4 超采样**：每个子扫描线收集与边的交点、按环绕数切出
//! 内部区间，再把覆盖率累加进当前行。抗锯齿靠超采样而不是后处理模糊，边缘是干净的。

use eframe::egui::{self, Color32, ColorImage, Rect, TextureHandle, pos2};

use std::f32::consts::TAU;

/// 微信图标（iconfont 导出的多色彩路径）。
pub const WECHAT_SVG: &str = include_str!("../../assets/wechat.svg");
/// QQ 图标（企鹅，多色）。
pub const QQ_SVG: &str = include_str!("../../assets/qq.svg");

// ---------------------------------------------------------------------------
// 对外：带缓存地取纹理 / 直接把图标画进矩形
// ---------------------------------------------------------------------------

/// 取一张已经光栅化好的纹理。
///
/// 缓存键里带了**像素尺寸**：同一个图标在侧栏（18px）和概览卡片（20px）下会被
/// 光栅化成两个不同分辨率的版本，各自都按 2× 采样，缩放时不会糊。
fn texture(ctx: &egui::Context, name: &str, svg: &str, px: usize) -> Option<TextureHandle> {
    let key = egui::Id::new(("svg-icon", name, px));
    if let Some(handle) = ctx.data(|d| d.get_temp::<TextureHandle>(key)) {
        return Some(handle);
    }
    let image = rasterize(svg, px)?;
    let handle = ctx.load_texture(name.to_owned(), image, egui::TextureOptions::LINEAR);
    ctx.data_mut(|d| d.insert_temp(key, handle.clone()));
    Some(handle)
}

/// 把 SVG 图标画进 `rect`（取短边，居中，等比）。
///
/// 图标保留自身的品牌配色，所以 `icons::paint` 里那条"颜色/线宽"对它们是无效的——
/// 微信的绿、QQ 的企鹅黄都得原样画出来，硬套成主题色反而认不出是谁。
pub fn paint(painter: &egui::Painter, rect: Rect, name: &str, svg: &str) {
    let side = rect.width().min(rect.height());
    if side < 2.0 {
        return;
    }
    // 按设备像素 2× 出图：够清晰，又不至于给每个尺寸都存一张大图
    let px = (side * painter.pixels_per_point() * 2.0)
        .round()
        .clamp(16.0, 256.0) as usize;
    let Some(tex) = texture(painter.ctx(), name, svg, px) else {
        return;
    };
    let size = tex.size_vec2();
    let scale = (side / size.x).min(side / size.y);
    let target = Rect::from_center_size(rect.center(), size * scale);
    painter.image(
        tex.id(),
        target,
        Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
        Color32::WHITE,
    );
}

// ---------------------------------------------------------------------------
// 光栅化
// ---------------------------------------------------------------------------

/// 一条子路径（已摊平成折线，坐标是 SVG 用户单位）。
struct SubPath {
    fill: [f32; 3],
    alpha: f32,
    even_odd: bool,
    contours: Vec<Vec<[f32; 2]>>,
}

/// 把 SVG 文本光栅化成 `px × px` 的 RGBA 图；解析不出任何路径时返回 `None`
/// （调用方会退回内置绘制，不至于白屏）。
pub fn rasterize(svg: &str, px: usize) -> Option<ColorImage> {
    let (vw, vh) = view_box(svg)?;
    let scale = px as f32 / vw.max(vh);

    // 曲线 / 弧的细分步长：约 0.6 设备像素一段，肉眼看不出折线
    let detail = (1.0 / scale * 0.6).max(0.05);

    let mut paths = Vec::new();
    for (d, fill, even_odd) in path_elements(svg) {
        let (rgb, alpha) = match fill.as_deref() {
            None => ([0.0, 0.0, 0.0], 1.0),
            Some(v) => match parse_color(v) {
                Some(c) => c,
                None => continue, // fill="none" 之类
            },
        };
        let mut contours = Vec::new();
        parse_path(&d, detail, &mut contours);
        if contours.is_empty() {
            continue;
        }
        paths.push(SubPath {
            fill: rgb,
            alpha,
            even_odd,
            contours,
        });
    }
    if paths.is_empty() {
        return None;
    }

    // 目标画布偏移：viewBox 不是正方形时居中（现用图标都是 1024×1024，走不到）
    let offset = [
        (px as f32 - vw * scale) / 2.0,
        (px as f32 - vh * scale) / 2.0,
    ];
    for path in &mut paths {
        for contour in &mut path.contours {
            for pt in contour.iter_mut() {
                pt[0] = pt[0] * scale + offset[0];
                pt[1] = pt[1] * scale + offset[1];
            }
        }
    }

    // 直通 alpha 的累积画布：每条子路径按文档顺序叠上去
    let mut canvas = vec![[0.0f32; 4]; px * px];
    for path in &paths {
        fill_sub_path(path, &mut canvas, px);
    }

    let mut pixels = Vec::with_capacity(px * px);
    for c in canvas {
        pixels.push(Color32::from_rgba_unmultiplied(
            to_u8(c[0]),
            to_u8(c[1]),
            to_u8(c[2]),
            to_u8(c[3]),
        ));
    }
    Some(ColorImage::new([px, px], pixels))
}

fn to_u8(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}

// ---------------------------------------------------------------------------
// 填充：扫描线 + 超采样
// ---------------------------------------------------------------------------

/// 每条像素在每个方向上取几个子采样点（4 就是 4×4 = 16 个采样）
const SS: usize = 4;

fn fill_sub_path(path: &SubPath, canvas: &mut [[f32; 4]], size: usize) {
    let w = size;
    let h = size;

    // 边表 + 包围盒
    let mut edges: Vec<(f32, f32, f32, f32, i32)> = Vec::new();
    let (mut min_x, mut min_y) = (f32::MAX, f32::MAX);
    let (mut max_x, mut max_y) = (f32::MIN, f32::MIN);
    for contour in &path.contours {
        if contour.len() < 2 {
            continue;
        }
        for i in 0..contour.len() {
            let a = contour[i];
            let b = contour[(i + 1) % contour.len()];
            for p in [a, b] {
                min_x = min_x.min(p[0]);
                min_y = min_y.min(p[1]);
                max_x = max_x.max(p[0]);
                max_y = max_y.max(p[1]);
            }
            if (a[1] - b[1]).abs() < 1e-6 {
                continue; // 水平边对扫描线没有贡献
            }
            let dir = if b[1] > a[1] { 1 } else { -1 };
            edges.push((a[0], a[1], b[0], b[1], dir));
        }
    }
    if edges.is_empty() {
        return;
    }

    let row_from = (min_y.floor().max(0.0)) as usize;
    let row_to = ((max_y.ceil()) as isize).clamp(0, h as isize) as usize;
    let col_from = (min_x.floor().max(0.0)) as usize;
    let col_to = ((max_x.ceil()) as isize).clamp(0, w as isize) as usize;

    // 一条像素行内，每个子扫描线累加"横向覆盖率"，最后统一除 SS
    let mut cov = vec![0.0f32; w];
    let mut hits: Vec<(f32, i32)> = Vec::with_capacity(64);

    for row in row_from..row_to {
        cov.iter_mut().for_each(|v| *v = 0.0);
        for sub in 0..SS {
            let y = row as f32 + (sub as f32 + 0.5) / SS as f32;
            hits.clear();
            for &(x0, y0, x1, y1, dir) in &edges {
                if (y0 <= y && y < y1) || (y1 <= y && y < y0) {
                    let t = (y - y0) / (y1 - y0);
                    hits.push((x0 + (x1 - x0) * t, dir));
                }
            }
            if hits.len() < 2 {
                continue;
            }
            hits.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));

            let mut winding = 0i32;
            let mut start: Option<f32> = None;
            for &(x, dir) in &hits {
                let was_inside = if path.even_odd {
                    winding % 2 != 0
                } else {
                    winding != 0
                };
                winding += if path.even_odd { 1 } else { dir };
                let now_inside = if path.even_odd {
                    winding % 2 != 0
                } else {
                    winding != 0
                };
                match (was_inside, now_inside) {
                    (false, true) => start = Some(x),
                    (true, false) => {
                        if let Some(s) = start.take() {
                            add_span(&mut cov, s, x, w);
                        }
                    }
                    _ => {}
                }
            }
        }

        let base = row * w;
        for col in col_from..col_to {
            let c = cov[col] / SS as f32;
            if c <= 0.002 {
                continue;
            }
            let dst = &mut canvas[base + col];
            blend(dst, path.fill, path.alpha * c);
        }
    }
}

/// 把 [a, b) 这一段宽度按像素拆成覆盖率累加（端点像素按小数部分计）。
fn add_span(cov: &mut [f32], mut a: f32, mut b: f32, w: usize) {
    if a > b {
        std::mem::swap(&mut a, &mut b);
    }
    let a = a.max(0.0);
    let b = b.min(w as f32);
    if b - a <= 1e-6 {
        return;
    }
    let ia = a.floor() as usize;
    let ib = (b.ceil() as usize).min(w);
    if ia >= w {
        return;
    }
    if ia + 1 >= ib {
        cov[ia] += b - a;
        return;
    }
    cov[ia] += ia as f32 + 1.0 - a;
    for slot in cov.iter_mut().take(ib - 1).skip(ia + 1) {
        *slot += 1.0;
    }
    cov[ib - 1] += b - (ib as f32 - 1.0);
}

/// 直通 alpha 的 "over" 合成。
fn blend(dst: &mut [f32; 4], rgb: [f32; 3], a: f32) {
    let da = dst[3];
    let out_a = a + da * (1.0 - a);
    if out_a <= 1e-6 {
        *dst = [0.0, 0.0, 0.0, 0.0];
        return;
    }
    for i in 0..3 {
        dst[i] = (rgb[i] * a + dst[i] * da * (1.0 - a)) / out_a;
    }
    dst[3] = out_a;
}

// ---------------------------------------------------------------------------
// SVG 文本解析
// ---------------------------------------------------------------------------

/// 读 `viewBox="0 0 1024 1024"`；没有就退化成整数字段的宽高。
fn view_box(svg: &str) -> Option<(f32, f32)> {
    if let Some(v) = attr(svg, "viewBox") {
        let nums: Vec<f32> = v
            .split(|c: char| c == ',' || c.is_whitespace())
            .filter(|s| !s.is_empty())
            .filter_map(|s| s.parse::<f32>().ok())
            .collect();
        if nums.len() == 4 && nums[2] > 0.0 && nums[3] > 0.0 {
            return Some((nums[2], nums[3]));
        }
    }
    let w = attr(svg, "width").and_then(|v| parse_len(v))?;
    let h = attr(svg, "height").and_then(|v| parse_len(v))?;
    Some((w, h))
}

fn parse_len(v: &str) -> Option<f32> {
    let trimmed = v.trim_end_matches(|c: char| c.is_ascii_alphabetic() || c == '%');
    trimmed.parse::<f32>().ok()
}

/// 取出所有 `<path>` 的 (d, fill, evenodd)。
fn path_elements(svg: &str) -> Vec<(String, Option<String>, bool)> {
    let mut out = Vec::new();
    let bytes = svg.as_bytes();
    let mut i = 0;
    while let Some(pos) = svg[i..].find("<path") {
        let start = i + pos;
        let Some(end_rel) = svg[start..].find('>') else {
            break;
        };
        let tag = &svg[start..start + end_rel];
        i = start + end_rel + 1;
        let Some(d) = attr(tag, "d") else { continue };
        let fill = attr(tag, "fill").map(|s| s.to_string());
        let even_odd = attr(tag, "fill-rule")
            .map(|s| s.trim().eq_ignore_ascii_case("evenodd"))
            .unwrap_or(false);
        let _ = bytes;
        out.push((d.to_string(), fill, even_odd));
    }
    out
}

/// 在标签文本里找 `name="值"` / `name='值'`。
///
/// 必须校验名字前面是空白，否则 `p-id="44737"` 会被当成 `d="…"` 命中。
fn attr<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let b = tag.as_bytes();
    let mut from = 0;
    while let Some(rel) = tag[from..].find(name) {
        let at = from + rel;
        let before_ok = at == 0 || b[at - 1].is_ascii_whitespace() || b[at - 1] == b'<';
        let mut k = at + name.len();
        if before_ok {
            while k < b.len() && b[k].is_ascii_whitespace() {
                k += 1;
            }
            if k < b.len() && b[k] == b'=' {
                k += 1;
                while k < b.len() && b[k].is_ascii_whitespace() {
                    k += 1;
                }
                if k < b.len() && (b[k] == b'"' || b[k] == b'\'') {
                    let quote = b[k];
                    let val_start = k + 1;
                    if let Some(end_rel) = b[val_start..].iter().position(|&c| c == quote) {
                        return Some(&tag[val_start..val_start + end_rel]);
                    }
                }
            }
        }
        from = at + name.len();
    }
    None
}

/// `#RGB` / `#RRGGBB` / `#RRGGBBAA` → (rgb 0..1, alpha)。
fn parse_color(v: &str) -> Option<([f32; 3], f32)> {
    let v = v.trim();
    let hex = v.strip_prefix('#')?;
    let hex_digit = |i: usize| -> Option<u8> {
        hex.as_bytes()
            .get(i)
            .and_then(|c| (*c as char).to_digit(16))
            .map(|d| d as u8)
    };
    let (r, g, b, a) = match hex.len() {
        3 => (
            hex_digit(0)? * 17,
            hex_digit(1)? * 17,
            hex_digit(2)? * 17,
            255,
        ),
        6 | 8 => (
            hex_digit(0)? * 16 + hex_digit(1)?,
            hex_digit(2)? * 16 + hex_digit(3)?,
            hex_digit(4)? * 16 + hex_digit(5)?,
            if hex.len() == 8 {
                hex_digit(6)? * 16 + hex_digit(7)?
            } else {
                255
            },
        ),
        _ => return None,
    };
    Some((
        [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0],
        a as f32 / 255.0,
    ))
}

// ---------------------------------------------------------------------------
// 路径数据 → 折线
// ---------------------------------------------------------------------------

struct Lexer<'a> {
    s: &'a [u8],
    i: usize,
}

impl<'a> Lexer<'a> {
    fn new(s: &'a str) -> Self {
        Self {
            s: s.as_bytes(),
            i: 0,
        }
    }

    fn skip_sep(&mut self) {
        while self.i < self.s.len() {
            let c = self.s[self.i];
            if c.is_ascii_whitespace() || c == b',' {
                self.i += 1;
            } else {
                break;
            }
        }
    }

    fn peek(&mut self) -> Option<u8> {
        self.skip_sep();
        self.s.get(self.i).copied()
    }

    fn bump(&mut self) {
        self.i += 1;
    }

    /// 读一个数（整数 / 小数 / 科学计数法）；读不到返回 `None`。
    fn num(&mut self) -> Option<f32> {
        self.skip_sep();
        let start = self.i;
        if self.i < self.s.len() && (self.s[self.i] == b'-' || self.s[self.i] == b'+') {
            self.i += 1;
        }
        let mut digits = false;
        while self.i < self.s.len() && self.s[self.i].is_ascii_digit() {
            self.i += 1;
            digits = true;
        }
        if self.i < self.s.len() && self.s[self.i] == b'.' {
            self.i += 1;
            while self.i < self.s.len() && self.s[self.i].is_ascii_digit() {
                self.i += 1;
                digits = true;
            }
        }
        if !digits {
            self.i = start;
            return None;
        }
        if self.i < self.s.len() && (self.s[self.i] == b'e' || self.s[self.i] == b'E') {
            let save = self.i;
            self.i += 1;
            if self.i < self.s.len() && (self.s[self.i] == b'-' || self.s[self.i] == b'+') {
                self.i += 1;
            }
            let mut exp_digits = false;
            while self.i < self.s.len() && self.s[self.i].is_ascii_digit() {
                self.i += 1;
                exp_digits = true;
            }
            if !exp_digits {
                self.i = save;
            }
        }
        std::str::from_utf8(&self.s[start..self.i])
            .ok()
            .and_then(|v| v.parse::<f32>().ok())
    }

    /// 弧命令的 large-arc / sweep 是**单字符布尔**，可能紧挨着写（如 `0150 50`），
    /// 不能用 `num` 读。
    fn flag(&mut self) -> Option<bool> {
        self.skip_sep();
        match self.s.get(self.i) {
            Some(b'0') => {
                self.i += 1;
                Some(false)
            }
            Some(b'1') => {
                self.i += 1;
                Some(true)
            }
            _ => None,
        }
    }
}

/// 路径数据 → 若干条折线（每条是一个闭合子路径）。
fn parse_path(d: &str, detail: f32, out: &mut Vec<Vec<[f32; 2]>>) {
    let mut lx = Lexer::new(d);
    let (mut x, mut y) = (0.0f32, 0.0f32);
    let (mut sx, mut sy) = (0.0f32, 0.0f32);
    let mut ctrl: Option<[f32; 2]> = None; // 上一个三次/二次曲线的末控制点（给 S/T 用）
    let mut cmd = 0u8;
    let mut cur: Vec<[f32; 2]> = Vec::new();

    loop {
        let Some(b) = lx.peek() else { break };
        if b.is_ascii_alphabetic() {
            cmd = b;
            lx.bump();
            if cmd == b'Z' || cmd == b'z' {
                if cur.len() > 1 {
                    out.push(std::mem::take(&mut cur));
                } else {
                    cur.clear();
                }
                x = sx;
                y = sy;
                ctrl = None;
                continue;
            }
        } else if cmd == 0 {
            break;
        }

        let rel = cmd.is_ascii_lowercase();
        let (ox, oy) = if rel { (x, y) } else { (0.0, 0.0) };

        match cmd.to_ascii_uppercase() {
            b'M' => {
                let (Some(nx), Some(ny)) = (lx.num(), lx.num()) else {
                    break;
                };
                if cur.len() > 1 {
                    out.push(std::mem::take(&mut cur));
                } else {
                    cur.clear();
                }
                x = ox + nx;
                y = oy + ny;
                sx = x;
                sy = y;
                cur.push([x, y]);
                ctrl = None;
                // 后续紧跟在 M 后面的坐标对按 lineto 处理
                cmd = if rel { b'l' } else { b'L' };
            }
            b'L' => {
                let (Some(nx), Some(ny)) = (lx.num(), lx.num()) else {
                    break;
                };
                x = ox + nx;
                y = oy + ny;
                cur.push([x, y]);
                ctrl = None;
            }
            b'H' => {
                let Some(nx) = lx.num() else { break };
                x = ox + nx;
                cur.push([x, y]);
                ctrl = None;
            }
            b'V' => {
                let Some(ny) = lx.num() else { break };
                y = oy + ny;
                cur.push([x, y]);
                ctrl = None;
            }
            b'C' => {
                let vals = match six(&mut lx) {
                    Some(v) => v,
                    None => break,
                };
                let p0 = [x, y];
                let c1 = [ox + vals[0], oy + vals[1]];
                let c2 = [ox + vals[2], oy + vals[3]];
                let p3 = [ox + vals[4], oy + vals[5]];
                flatten_cubic(p0, c1, c2, p3, detail, &mut cur);
                x = p3[0];
                y = p3[1];
                ctrl = Some(c2);
            }
            b'S' => {
                let vals = match four(&mut lx) {
                    Some(v) => v,
                    None => break,
                };
                let p0 = [x, y];
                let c1 = match ctrl {
                    Some(c) => [2.0 * x - c[0], 2.0 * y - c[1]],
                    None => p0,
                };
                let c2 = [ox + vals[0], oy + vals[1]];
                let p3 = [ox + vals[2], oy + vals[3]];
                flatten_cubic(p0, c1, c2, p3, detail, &mut cur);
                x = p3[0];
                y = p3[1];
                ctrl = Some(c2);
            }
            b'Q' => {
                let vals = match four(&mut lx) {
                    Some(v) => v,
                    None => break,
                };
                let p0 = [x, y];
                let c = [ox + vals[0], oy + vals[1]];
                let p2 = [ox + vals[2], oy + vals[3]];
                flatten_quad(p0, c, p2, detail, &mut cur);
                x = p2[0];
                y = p2[1];
                ctrl = Some(c);
            }
            b'T' => {
                let (Some(nx), Some(ny)) = (lx.num(), lx.num()) else {
                    break;
                };
                let p0 = [x, y];
                let c = match ctrl {
                    Some(p) => [2.0 * x - p[0], 2.0 * y - p[1]],
                    None => p0,
                };
                let p2 = [ox + nx, oy + ny];
                flatten_quad(p0, c, p2, detail, &mut cur);
                x = p2[0];
                y = p2[1];
                ctrl = Some(c);
            }
            b'A' => {
                let (Some(rx), Some(ry), Some(rot)) = (lx.num(), lx.num(), lx.num()) else {
                    break;
                };
                let (Some(large), Some(sweep)) = (lx.flag(), lx.flag()) else {
                    break;
                };
                let (Some(nx), Some(ny)) = (lx.num(), lx.num()) else {
                    break;
                };
                let p1 = [ox + nx, oy + ny];
                flatten_arc([x, y], rx, ry, rot, large, sweep, p1, detail, &mut cur);
                x = p1[0];
                y = p1[1];
                ctrl = None;
            }
            _ => break,
        }
    }

    if cur.len() > 1 {
        out.push(cur);
    }
}

fn four(lx: &mut Lexer) -> Option<[f32; 4]> {
    Some([lx.num()?, lx.num()?, lx.num()?, lx.num()?])
}

fn six(lx: &mut Lexer) -> Option<[f32; 6]> {
    Some([
        lx.num()?,
        lx.num()?,
        lx.num()?,
        lx.num()?,
        lx.num()?,
        lx.num()?,
    ])
}

fn flatten_cubic(
    p0: [f32; 2],
    p1: [f32; 2],
    p2: [f32; 2],
    p3: [f32; 2],
    detail: f32,
    out: &mut Vec<[f32; 2]>,
) {
    // 用控制多边形长度估段数：比按弦长估稳，回环曲线也不会切少
    let len = dist(p0, p1) + dist(p1, p2) + dist(p2, p3);
    let steps = ((len / detail).ceil() as usize).clamp(2, 64);
    for i in 1..=steps {
        let t = i as f32 / steps as f32;
        let u = 1.0 - t;
        let b0 = u * u * u;
        let b1 = 3.0 * u * u * t;
        let b2 = 3.0 * u * t * t;
        let b3 = t * t * t;
        out.push([
            b0 * p0[0] + b1 * p1[0] + b2 * p2[0] + b3 * p3[0],
            b0 * p0[1] + b1 * p1[1] + b2 * p2[1] + b3 * p3[1],
        ]);
    }
}

fn flatten_quad(p0: [f32; 2], p1: [f32; 2], p2: [f32; 2], detail: f32, out: &mut Vec<[f32; 2]>) {
    let len = dist(p0, p1) + dist(p1, p2);
    let steps = ((len / detail).ceil() as usize).clamp(2, 64);
    for i in 1..=steps {
        let t = i as f32 / steps as f32;
        let u = 1.0 - t;
        out.push([
            u * u * p0[0] + 2.0 * u * t * p1[0] + t * t * p2[0],
            u * u * p0[1] + 2.0 * u * t * p1[1] + t * t * p2[1],
        ]);
    }
}

fn dist(a: [f32; 2], b: [f32; 2]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt()
}

/// 椭圆弧：按 SVG 规范的"端点 → 圆心"参数化，再按弧长细分。
#[allow(clippy::too_many_arguments)]
fn flatten_arc(
    p0: [f32; 2],
    rx_in: f32,
    ry_in: f32,
    rot_deg: f32,
    large: bool,
    sweep: bool,
    p1: [f32; 2],
    detail: f32,
    out: &mut Vec<[f32; 2]>,
) {
    let (mut rx, mut ry) = (rx_in.abs(), ry_in.abs());
    if rx < 1e-6 || ry < 1e-6 || (p0[0] - p1[0]).abs() < 1e-9 && (p0[1] - p1[1]).abs() < 1e-9 {
        out.push(p1);
        return;
    }
    let phi = rot_deg.to_radians();
    let (sin_phi, cos_phi) = phi.sin_cos();

    let dx2 = (p0[0] - p1[0]) / 2.0;
    let dy2 = (p0[1] - p1[1]) / 2.0;
    let x1p = cos_phi * dx2 + sin_phi * dy2;
    let y1p = -sin_phi * dx2 + cos_phi * dy2;

    // 半径太小放不下这段弧时按规范放大到刚好够
    let lambda = x1p * x1p / (rx * rx) + y1p * y1p / (ry * ry);
    if lambda > 1.0 {
        let s = lambda.sqrt();
        rx *= s;
        ry *= s;
    }

    let num = (rx * rx * ry * ry - rx * rx * y1p * y1p - ry * ry * x1p * x1p).max(0.0);
    let den = rx * rx * y1p * y1p + ry * ry * x1p * x1p;
    let coef = if den <= 1e-12 {
        0.0
    } else {
        let s = (num / den).sqrt();
        if large != sweep { s } else { -s }
    };
    let cxp = coef * rx * y1p / ry;
    let cyp = -coef * ry * x1p / rx;
    let cx = cos_phi * cxp - sin_phi * cyp + (p0[0] + p1[0]) / 2.0;
    let cy = sin_phi * cxp + cos_phi * cyp + (p0[1] + p1[1]) / 2.0;

    let angle = |ux: f32, uy: f32, vx: f32, vy: f32| -> f32 {
        let dot = ux * vx + uy * vy;
        let len = ((ux * ux + uy * uy) * (vx * vx + vy * vy)).sqrt();
        let mut a = if len <= 1e-12 {
            0.0
        } else {
            (dot / len).clamp(-1.0, 1.0).acos()
        };
        if ux * vy - uy * vx < 0.0 {
            a = -a;
        }
        a
    };

    let ux = (x1p - cxp) / rx;
    let uy = (y1p - cyp) / ry;
    let vx = (-x1p - cxp) / rx;
    let vy = (-y1p - cyp) / ry;
    let theta1 = angle(1.0, 0.0, ux, uy);
    let mut dtheta = angle(ux, uy, vx, vy);
    if !sweep && dtheta > 0.0 {
        dtheta -= TAU;
    } else if sweep && dtheta < 0.0 {
        dtheta += TAU;
    }

    let steps = ((dtheta.abs() * rx.max(ry)) / detail).ceil() as usize;
    let steps = steps.clamp(2, 128);
    for k in 1..=steps {
        let t = theta1 + dtheta * (k as f32 / steps as f32);
        let (sin_t, cos_t) = t.sin_cos();
        out.push([
            cx + rx * cos_t * cos_phi - ry * sin_t * sin_phi,
            cy + rx * cos_t * sin_phi + ry * sin_t * cos_phi,
        ]);
    }
}

/// 仅供单元测试：把图标画到控制台字符画，肉眼确认光栅化没画歪。
/// 大写字母表示主色（K 黑 / W 白 / R 红 / Y 黄 / G 绿），`+` 是混色或半透明。
#[cfg(test)]
fn debug_art(svg: &str, size: usize) -> String {
    let img = rasterize(svg, size).expect("应当能解析出路径");
    let mut s = String::new();
    for y in 0..size {
        for x in 0..size {
            let p = img.pixels[y * size + x];
            let a = p.a();
            let ch = if a <= 40 {
                ' '
            } else if a < 200 {
                '+'
            } else {
                let (r, g, b) = (p.r(), p.g(), p.b());
                if r > 200 && g > 200 && b > 200 {
                    'W'
                } else if r > 180 && g < 120 && b < 120 {
                    'R'
                } else if r > 180 && g > 140 && b < 120 {
                    'Y'
                } else if g > 150 && r < 120 {
                    'G'
                } else if r < 90 && g < 90 && b < 90 {
                    'K'
                } else {
                    '?'
                }
            };
            s.push(ch);
        }
        s.push('\n');
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wechat_icon_covers_center() {
        let img = rasterize(WECHAT_SVG, 64).expect("微信图标应能解析");
        assert_eq!(img.size, [64, 64]);
        // 图标本身有留白（viewBox 里内容不是满格），但中心附近必须有像素
        let mut opaque = 0;
        for p in &img.pixels {
            if p.a() > 128 {
                opaque += 1;
            }
        }
        assert!(
            opaque > 64 * 64 / 20,
            "画出来的像素太少（{opaque}），解析可能失败"
        );
    }

    #[test]
    fn qq_icon_has_multiple_colors() {
        let img = rasterize(QQ_SVG, 64).expect("QQ 图标应能解析");
        let mut colors: Vec<u32> = img
            .pixels
            .iter()
            .filter(|p| p.a() > 200)
            .map(|p| (u32::from(p.r()) << 16) | (u32::from(p.g()) << 8) | u32::from(p.b()))
            .collect();
        colors.sort_unstable();
        colors.dedup();
        assert!(colors.len() > 3, "QQ 是多色图标，颜色种类应当不止于此");
    }

    #[test]
    fn view_box_and_attr_parsing() {
        assert_eq!(view_box(WECHAT_SVG), Some((1024.0, 1024.0)));
        let tag = r##"<path d="M0 0L1 1" fill="#fff" p-id="44737"></path>"##;
        assert_eq!(attr(tag, "d"), Some("M0 0L1 1"));
        assert_eq!(attr(tag, "fill"), Some("#fff"));
        // `p-id` 不能被当成 `d`
        assert_eq!(attr(r#"<path p-id="44737" d="M0 0"/>"#, "d"), Some("M0 0"));
    }

    #[test]
    fn implicit_lineto_after_move() {
        let mut out = Vec::new();
        parse_path("M0 0 10 0 10 10Z", 0.5, &mut out);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].len(), 3);
    }

    #[test]
    fn arc_hits_endpoint() {
        let mut out = Vec::new();
        parse_path("M0 0 A10 10 0 0 1 20 0", 0.5, &mut out);
        let last = *out[0].last().unwrap();
        assert!((last[0] - 20.0).abs() < 0.01, "弧终点 x 偏了：{last:?}");
        assert!(last[1].abs() < 0.01, "弧终点 y 偏了：{last:?}");
    }

    /// 手工看一眼字符画（`DUMP_ICON_ART=wechat|qq cargo test render_artifacts -- --nocapture`）。
    #[test]
    fn render_artifacts() {
        match std::env::var("DUMP_ICON_ART").as_deref() {
            Ok("wechat") => println!("{}", debug_art(WECHAT_SVG, 44)),
            Ok("qq") => println!("{}", debug_art(QQ_SVG, 44)),
            _ => {}
        }
    }
}
