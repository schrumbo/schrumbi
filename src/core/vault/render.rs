use crate::core::pdf_export;

/// The helpers MiTeX's output calls; the rest maps onto Typst's own math.
/// A formula as SVG in the given color, with its size in points.
pub fn math_svg(latex: &str, display: bool, color: &str) -> Result<(String, f32, f32), String> {
    let (owned, color) = (latex.to_string(), color.to_string());
    std::panic::catch_unwind(move || draw_math(&owned, display, &color)).unwrap_or_else(|_| Err("Diese Formel lässt sich nicht zeichnen.".to_string()))
}

fn draw_math(latex: &str, display: bool, color: &str) -> Result<(String, f32, f32), String> {
    let formula = typst_formula(latex, display)?;
    let source = format!("#set page(width: auto, height: auto, margin: 1pt, fill: none)\n#set text(top-edge: \"bounds\", bottom-edge: \"bounds\")\n#set text(size: 14pt, fill: rgb(\"{color}\"))\n#{formula}\n");
    pdf_export::typst_svg(&source)
}

/// A formula as Typst markup, for the PDF export.
pub fn math_markup(latex: &str, display: bool) -> Result<String, String> {
    let owned = latex.to_string();
    std::panic::catch_unwind(move || typst_formula(&owned, display).map(|f| format!("#{f}"))).unwrap_or_else(|_| Err("Diese Formel lässt sich nicht zeichnen.".to_string()))
}

/// The converter writes some symbols under names Typst has since changed.
fn current_typst_names(body: &str) -> String {
    static OLD: std::sync::LazyLock<Vec<(regex::Regex, &str)>> = std::sync::LazyLock::new(|| {
        [(r"\bsect\b", "inter"), (r"\bdiff\b", "partial"), (r"\bplus\.circle\b", "plus.o"), (r"\btimes\.circle\b", "times.o"), (r"\bdot\.circle\b", "dot.o"), (r"\bminus\.circle\b", "minus.o"), (r"\bplanck\.reduce\b", "planck"), (r"\bangle\.l\b", "chevron.l"), (r"\bangle\.r\b", "chevron.r")]
            .into_iter()
            .map(|(old, new)| (regex::Regex::new(old).unwrap(), new))
            .collect()
    });
    let mut out = body.to_string();
    for (old, new) in OLD.iter() {
        if old.is_match(&out) {
            out = old.replace_all(&out, *new).into_owned();
        }
    }
    out
}

const ARRAY_RULES: &str = r#"let mitexarray(arg0: "l", ..args) = {
    if args.pos().len() == 0 {
      return
    }
    let spec = if type(arg0) == str { arg0 } else { get-tex-str(arg0) }
    let aligns = ()
    let vlines = ()
    let hlines = ()
    for ch in spec.clusters() {
      if ch == "|" {
        vlines.push(aligns.len())
      } else if ch in ("l", "c", "r") {
        aligns.push(ch)
      } else if ch >= "A" and ch <= "Z" {
        hlines.push(str.to-unicode(ch) - 65)
      }
    }
    if aligns.len() == 0 {
      aligns = ("l",)
    }
    let matrix = args.pos().map(row => if type(row) == array { row } else { (row,) })
    let n = matrix.len()
    let m = calc.max(..matrix.map(row => row.len()))
    matrix = matrix.map(row => row + (m - row.len()) * (none,))
    set align(("l": left, "c": center, "r": right).at(aligns.at(0)))
    let cells = matrix.flatten().map(it => $it$)
    if vlines.len() == 0 and hlines.len() == 0 {
      pad(y: 0.2em, grid(columns: m, column-gutter: 0.5em, row-gutter: 0.5em, ..cells))
    } else {
      context {
        let rule = 0.5pt + text.fill
        pad(y: 0.2em, grid(
          columns: m,
          inset: (x: 0.25em, y: 0.25em),
          stroke: (x, y) => (
            left: if x in vlines { rule },
            right: if x == m - 1 and m in vlines { rule },
            top: if y in hlines { rule },
            bottom: if y == n - 1 and n in hlines { rule },
          ),
          ..cells
        ))
      }
    }
  }"#;

/// What the converter wrote, in words for the reader: the command or environment it does not know.
fn readable_error(error: &str) -> String {
    let named = |marker: &str| error.split_once(marker).and_then(|(_, rest)| rest.split_whitespace().next()).map(|name| name.trim_start_matches('\\').to_string());
    if let Some(name) = named("unknown command: ") {
        return crate::core::i18n::trf("Unbekannter Befehl: %@", &[&format!("\\{name}")]);
    }
    if let Some(name) = named("unknown environment: ") {
        return crate::core::i18n::trf("Unbekannte Umgebung: %@", &[&name]);
    }
    error.trim_start_matches("error: ").to_string()
}

/// A Typst code block that draws the formula with every LaTeX command the converter knows.
fn typst_formula(latex: &str, display: bool) -> Result<String, String> {
    let prepared = super::latex::prepare(latex);
    let body = current_typst_names(&mitex::convert_math(&prepared, None).map_err(|e| readable_error(&e))?);
    let quoted = format!("${body}$").replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n").replace('\r', "");
    Ok(format!("{{ import \"/mitex.typ\": mitex-scope, get-tex-str; {ARRAY_RULES}; math.equation(block: {display}, eval(\"{quoted}\", scope: mitex-scope + (mitexarray: mitexarray))) }}"))
}

/// A Mermaid diagram as SVG.
pub fn mermaid_svg(code: &str, dark: bool) -> Result<String, String> {
    if let Ok(Ok(parsed)) = std::panic::catch_unwind(|| mermaid_rs_renderer::parse_mermaid(code))
        && parsed.graph.kind == mermaid_rs_renderer::DiagramKind::Pie
        && let Some(svg) = pie_svg(parsed.graph.pie_title.as_deref(), parsed.graph.pie_show_data, &parsed.graph.pie_slices.iter().map(|s| (s.label.clone(), s.value)).collect::<Vec<_>>(), dark)
    {
        return Ok(svg);
    }
    let mut theme = if dark { mermaid_rs_renderer::Theme::dark() } else { mermaid_rs_renderer::Theme::modern() };
    let (fill, border, line, text) = if dark { ("#2B3442", "#82AAF5", "#8E939B", "#EDEDED") } else { ("#E8EEF9", "#1F5FCC", "#68707C", "#1F1F1F") };
    theme.primary_color = fill.into();
    theme.primary_border_color = border.into();
    theme.primary_text_color = text.into();
    theme.line_color = line.into();
    theme.text_color = text.into();
    theme.background = "transparent".into();
    theme.edge_label_background = "transparent".into();
    theme.pie_colors = if dark { ["#5B8DEF", "#E5A76B", "#7CC7A0", "#C792EA", "#EF7B7B", "#62C5D6", "#E2C65F", "#9A8CFF", "#F08FC0", "#8FA3BF", "#A5C76B", "#D49A6A"] } else { ["#2F6FDE", "#D9822B", "#2E9E6A", "#8E5BD0", "#D64545", "#1E9BB0", "#BFA01C", "#5B54D6", "#D6559A", "#5F7388", "#6F9E2B", "#A9692F"] }.map(String::from);
    theme.pie_section_text_color = "#FFFFFF".into();
    theme.pie_title_text_color = text.into();
    theme.pie_legend_text_color = text.into();
    theme.pie_stroke_color = "transparent".into();
    theme.sequence_actor_fill = fill.into();
    theme.sequence_actor_border = border.into();
    theme.sequence_actor_line = line.into();
    theme.sequence_activation_fill = fill.into();
    theme.sequence_activation_border = border.into();
    theme.sequence_note_fill = if dark { "#3A3426" } else { "#FFF6D6" }.into();
    theme.sequence_note_border = if dark { "#C9A34A" } else { "#D9B44A" }.into();
    theme.pie_outer_stroke_color = "transparent".into();
    let options = mermaid_rs_renderer::RenderOptions { theme, ..Default::default() };
    let caught = std::panic::catch_unwind(|| mermaid_rs_renderer::render_with_options(code, options));
    match caught {
        Ok(Ok(svg)) => Ok(if dark { darken_light_fills(svg) } else { svg }),
        Ok(Err(e)) => Err(e.to_string()),
        Err(_) => Err("Das Diagramm ließ sich nicht zeichnen.".to_string()),
    }
}

/// Series colours for light and dark notes, checked for colour blindness against each surface.
const PIE_LIGHT: [&str; 8] = ["#2A78D6", "#EB6834", "#1BAF7A", "#EDA100", "#E87BA4", "#008300", "#6250D6", "#E34948"];
const PIE_DARK: [&str; 8] = ["#3987E5", "#D95926", "#199E70", "#C98500", "#D55181", "#008300", "#9085E9", "#E66767"];

fn xml_escaped(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// About how wide text is in the diagram font, for laying out the legend.
fn text_width(text: &str, size: f32) -> f32 {
    text.chars().map(|c| if (c as u32) >= 0x1100 && !matches!(c as u32, 0x2000..=0x206F) { 1. } else if c.is_uppercase() || c == 'm' || c == 'w' { 0.72 } else if c == ' ' || c == 'i' || c == 'l' || c == '.' || c == ',' { 0.3 } else { 0.58 }).sum::<f32>() * size
}

fn shortened(text: &str, most: usize) -> String {
    if text.chars().count() <= most { text.to_string() } else { format!("{}…", text.chars().take(most - 1).collect::<String>().trim_end()) }
}

fn number(value: f32) -> String {
    let shown = if (value - value.round()).abs() < 0.005 { format!("{}", value.round() as i64) } else { format!("{value:.2}").trim_end_matches('0').trim_end_matches('.').to_string() };
    if crate::core::i18n::is_english() { shown } else { shown.replace('.', ",") }
}

fn percent(share: f32) -> String {
    let n = share * 100.;
    let shown = if n > 0. && n < 1. { "<1".to_string() } else { format!("{}", n.round() as i64) };
    if crate::core::i18n::is_english() { format!("{shown}%") } else { format!("{shown}\u{202F}%") }
}

/// Whether dark text reads better than white on a fill.
fn light_fill(hex: &str) -> bool {
    let value = u32::from_str_radix(hex.trim_start_matches('#'), 16).unwrap_or(0);
    let channel = |shift: u32| {
        let c = ((value >> shift) & 0xFF) as f32 / 255.;
        if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
    };
    0.2126 * channel(16) + 0.7152 * channel(8) + 0.0722 * channel(0) > 0.4
}

/// A pie chart as a ring with gaps between slices and a legend that carries the numbers.
fn pie_svg(title: Option<&str>, show_data: bool, slices: &[(String, f32)], dark: bool) -> Option<String> {
    let mut slices: Vec<(String, f32)> = slices.iter().filter(|(_, v)| v.is_finite() && *v > 0.).cloned().collect();
    let total: f32 = slices.iter().map(|(_, v)| v).sum();
    if slices.is_empty() || total <= 0. {
        return None;
    }
    let palette = if dark { PIE_DARK } else { PIE_LIGHT };
    if slices.len() > palette.len() {
        let mut order: Vec<usize> = (0..slices.len()).collect();
        order.sort_by(|a, b| slices[*b].1.total_cmp(&slices[*a].1));
        let kept: std::collections::HashSet<usize> = order.into_iter().take(palette.len() - 1).collect();
        let rest: f32 = slices.iter().enumerate().filter(|(i, _)| !kept.contains(i)).map(|(_, (_, v))| v).sum();
        slices = slices.into_iter().enumerate().filter(|(i, _)| kept.contains(i)).map(|(_, s)| s).collect();
        slices.push((crate::core::i18n::tr("Andere"), rest));
    }
    let (text, muted, other) = if dark { ("#E4E4E4", "#A7A7A7", "#6E6E73") } else { ("#1F1F1F", "#616161", "#A1A1A6") };
    let folded = slices.len() == palette.len() && slices.len() > 1 && slices.last().is_some_and(|(l, _)| *l == crate::core::i18n::tr("Andere"));
    let colour = |i: usize| if folded && i == slices.len() - 1 { other } else { palette[i % palette.len()] };
    let (outer, inner, gap) = (92_f32, 58_f32, 2_f32);
    let (legend_size, row, swatch) = (13_f32, 24_f32, 10_f32);
    let labels: Vec<String> = slices.iter().map(|(l, _)| shortened(l, 32)).collect();
    let values: Vec<String> = slices.iter().map(|(_, v)| number(*v)).collect();
    let shares: Vec<String> = slices.iter().map(|(_, v)| percent(v / total)).collect();
    let label_width = labels.iter().map(|l| text_width(l, legend_size)).fold(0., f32::max);
    let value_width = if show_data { values.iter().map(|v| text_width(v, legend_size)).fold(0., f32::max) + 16. } else { 0. };
    let share_width = shares.iter().map(|s| text_width(s, legend_size)).fold(0., f32::max);
    let legend_width = swatch + 8. + label_width + 20. + value_width + share_width;
    let legend_height = slices.len() as f32 * row;
    let title_size = 15_f32;
    let top = if title.is_some() { title_size * 1.4 + 16. } else { 0. };
    let body = (outer * 2.).max(legend_height);
    let pad = 4_f32;
    let content = outer * 2. + 32. + legend_width;
    let width = (pad * 2. + content).max(title.map_or(0., |t| text_width(&shortened(t, 60), title_size) + pad * 2.));
    let height = pad * 2. + top + body;
    let (cx, cy) = ((width - content) / 2. + outer, pad + top + body / 2.);
    let font = r#"font-family="-apple-system, 'SF Pro Text', 'Helvetica Neue', Inter, 'Segoe UI', Arial, sans-serif""#;
    let mut svg = format!(r#"<svg xmlns="http://www.w3.org/2000/svg" width="{width:.0}" height="{height:.0}" viewBox="0 0 {width:.0} {height:.0}" {font}>"#);
    if let Some(title) = title {
        svg.push_str(&format!(r#"<text x="{:.1}" y="{:.1}" text-anchor="middle" font-size="{title_size}" font-weight="600" fill="{text}">{}</text>"#, width / 2., pad + title_size, xml_escaped(&shortened(title, 60))));
    }
    let point = |angle: f32, radius: f32| (cx + radius * angle.sin(), cy - radius * angle.cos());
    let tau = std::f32::consts::TAU;
    if slices.len() == 1 {
        svg.push_str(&format!(r#"<path fill-rule="evenodd" fill="{}" d="M {cx} {:.2} a {outer} {outer} 0 1 0 0.01 0 Z M {cx} {:.2} a {inner} {inner} 0 1 0 0.01 0 Z"/>"#, colour(0), cy - outer, cy - inner));
    }
    let mut start = 0_f32;
    let mut marks = String::new();
    for (i, (_, value)) in slices.iter().enumerate() {
        let sweep = value / total * tau;
        let end = start + sweep;
        if slices.len() > 1 {
            let (outer_pad, inner_pad) = ((gap / 2.) / outer, (gap / 2.) / inner);
            let (outer_pad, inner_pad) = if sweep > inner_pad * 2. + 0.002 { (outer_pad, inner_pad) } else { (0., 0.) };
            let (a, b) = (point(start + outer_pad, outer), point(end - outer_pad, outer));
            let (c, d) = (point(end - inner_pad, inner), point(start + inner_pad, inner));
            let large_outer = i32::from(sweep - outer_pad * 2. > std::f32::consts::PI);
            let large_inner = i32::from(sweep - inner_pad * 2. > std::f32::consts::PI);
            svg.push_str(&format!(
                r#"<path fill="{}" d="M {:.2} {:.2} A {outer} {outer} 0 {large_outer} 1 {:.2} {:.2} L {:.2} {:.2} A {inner} {inner} 0 {large_inner} 0 {:.2} {:.2} Z"/>"#,
                colour(i), a.0, a.1, b.0, b.1, c.0, c.1, d.0, d.1
            ));
        }
        if value / total >= 0.08 {
            let (x, y) = point(start + sweep / 2., (outer + inner) / 2.);
            let ink = if light_fill(colour(i)) { "#1A1A19" } else { "#FFFFFF" };
            marks.push_str(&format!(r#"<text x="{x:.1}" y="{:.1}" text-anchor="middle" font-size="11" font-weight="600" fill="{ink}">{}</text>"#, y + 4., xml_escaped(&shares[i])));
        }
        start = end;
    }
    svg.push_str(&marks);
    let left = cx + outer + 32.;
    let first = cy - legend_height / 2.;
    for (i, label) in labels.iter().enumerate() {
        let middle = first + row * (i as f32 + 0.5);
        let baseline = middle + legend_size * 0.36;
        svg.push_str(&format!(r#"<rect x="{left:.1}" y="{:.1}" width="{swatch}" height="{swatch}" rx="3" fill="{}"/>"#, middle - swatch / 2., colour(i)));
        svg.push_str(&format!(r#"<text x="{:.1}" y="{baseline:.1}" font-size="{legend_size}" fill="{text}">{}</text>"#, left + swatch + 8., xml_escaped(label)));
        let right = left + legend_width;
        if show_data {
            svg.push_str(&format!(r#"<text x="{:.1}" y="{baseline:.1}" text-anchor="end" font-size="{legend_size}" fill="{muted}">{}</text>"#, right - share_width - 16., xml_escaped(&values[i])));
        }
        svg.push_str(&format!(r#"<text x="{right:.1}" y="{baseline:.1}" text-anchor="end" font-size="{legend_size}" font-weight="500" fill="{muted}">{}</text>"#, xml_escaped(&shares[i])));
    }
    svg.push_str("</svg>");
    Some(svg)
}

/// Quadrant charts and timelines come with fixed light colours, whatever the theme says.
fn darken_light_fills(svg: String) -> String {
    let mut out = svg;
    for (light, dark) in [("#ECECFF", "#2B3442"), ("#f1f1ff", "#2B3442"), ("#f6f6ff", "#2F3847"), ("#fbfbff", "#333C4C"), ("#c7c7f1", "#82AAF5"), ("#131300", "#EDEDED")] {
        out = out.replace(light, dark);
    }
    out
}

/// Node labels of a Mermaid diagram marked `internal-link`, which lead to notes.
pub fn mermaid_links(code: &str) -> Vec<String> {
    static CLASS: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| regex::Regex::new(r"(?m)^\s*class\s+(.+?)\s+internal-link\s*;?\s*$").unwrap());
    let mut out = Vec::new();
    for c in CLASS.captures_iter(code) {
        for id in c[1].split(',') {
            let id = id.trim().trim_matches('"');
            let label = regex::Regex::new(&format!(r#"(?m)\b{}\s*[\[\(\{{]+"?([^"\]\)\}}]+)"#, regex::escape(id))).ok().and_then(|r| r.captures(code).map(|x| x[1].trim().to_string()));
            out.push(label.unwrap_or_else(|| id.to_string()));
        }
    }
    out
}

/// Where the labels of linked nodes stand in a drawn diagram, in the picture's own coordinates.
pub fn mermaid_link_spots(svg: &str, links: &[String]) -> Vec<(String, f32, f32)> {
    static LABEL: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| regex::Regex::new(r#"(?s)<text\b([^>]*)>(.*?)</text>"#).unwrap());
    static TAG: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| regex::Regex::new(r"<[^>]+>").unwrap());
    let attribute = |attributes: &str, name: &str| -> Option<f32> {
        let start = attributes.find(&format!(" {name}=\""))? + name.len() + 3;
        let end = attributes[start..].find('"')? + start;
        attributes[start..end].parse().ok()
    };
    let number = |name: &str| -> Option<f32> {
        let start = svg.find(&format!("{name}=\""))? + name.len() + 2;
        svg[start..].split('"').next()?.trim_end_matches("pt").trim_end_matches("px").parse().ok()
    };
    let view: Vec<f32> = svg.find("viewBox=\"").and_then(|at| svg[at + 9..].split('"').next()).map(|v| v.split([' ', ',']).filter_map(|n| n.parse().ok()).collect()).unwrap_or_default();
    let (left, top, x_scale, y_scale) = match (view.as_slice(), number("width"), number("height")) {
        ([x, y, w, h], Some(width), Some(height)) if *w > 0. && *h > 0. => (*x, *y, width / w, height / h),
        _ => (0., 0., 1., 1.),
    };
    let mut spots = Vec::new();
    for c in LABEL.captures_iter(svg) {
        let shown = TAG.replace_all(&c[2], "");
        let shown = shown.trim().replace("&amp;", "&");
        if let Some(link) = links.iter().find(|l| l.as_str() == shown)
            && let (Some(x), Some(y)) = (attribute(&c[1], "x"), attribute(&c[1], "y"))
        {
            spots.push((link.clone(), (x - left) * x_scale, (y - top) * y_scale));
        }
    }
    spots
}

/// A formula in running text as plain characters, where that is possible; text has no room for an image.
pub fn math_inline(latex: &str) -> Option<String> {
    let mut out = latex.trim().to_string();
    for (from, to) in COMMANDS {
        out = out.replace(from, to);
    }
    static FRAC: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| regex::Regex::new(r"\\frac\{([^{}]*)\}\{([^{}]*)\}").unwrap());
    static SQRT: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| regex::Regex::new(r"\\sqrt\{([^{}]*)\}").unwrap());
    static TEXT: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| regex::Regex::new(r"\\(?:text|mathrm|mathbf|mathit)\{([^{}]*)\}").unwrap());
    out = FRAC.replace_all(&out, "($1)/($2)").to_string();
    out = SQRT.replace_all(&out, "√($1)").to_string();
    out = TEXT.replace_all(&out, "$1").to_string();
    let mut result = String::with_capacity(out.len());
    let chars: Vec<char> = out.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            marker @ ('^' | '_') => {
                let map = if marker == '^' { script_up as fn(char) -> Option<char> } else { script_down };
                i += 1;
                let group: Vec<char> = if chars.get(i) == Some(&'{') {
                    let end = chars[i..].iter().position(|c| *c == '}')?;
                    let g = chars[i + 1..i + end].to_vec();
                    i += end + 1;
                    g
                } else {
                    let g = vec![*chars.get(i)?];
                    i += 1;
                    g
                };
                for c in group {
                    result.push(map(c)?);
                }
            }
            '{' | '}' => i += 1,
            c => {
                result.push(c);
                i += 1;
            }
        }
    }
    (!result.contains('\\')).then_some(result)
}

const COMMANDS: &[(&str, &str)] = &[
    ("\\alpha", "α"), ("\\beta", "β"), ("\\gamma", "γ"), ("\\delta", "δ"), ("\\epsilon", "ε"), ("\\varepsilon", "ε"), ("\\zeta", "ζ"), ("\\eta", "η"), ("\\theta", "θ"), ("\\iota", "ι"), ("\\kappa", "κ"), ("\\lambda", "λ"), ("\\mu", "μ"), ("\\nu", "ν"), ("\\xi", "ξ"), ("\\pi", "π"), ("\\rho", "ρ"), ("\\sigma", "σ"), ("\\tau", "τ"), ("\\phi", "φ"), ("\\varphi", "φ"), ("\\chi", "χ"), ("\\psi", "ψ"), ("\\omega", "ω"),
    ("\\Gamma", "Γ"), ("\\Delta", "Δ"), ("\\Theta", "Θ"), ("\\Lambda", "Λ"), ("\\Xi", "Ξ"), ("\\Pi", "Π"), ("\\Sigma", "Σ"), ("\\Phi", "Φ"), ("\\Psi", "Ψ"), ("\\Omega", "Ω"),
    ("\\times", "×"), ("\\cdot", "·"), ("\\div", "÷"), ("\\pm", "±"), ("\\mp", "∓"), ("\\leq", "≤"), ("\\le", "≤"), ("\\geq", "≥"), ("\\ge", "≥"), ("\\neq", "≠"), ("\\ne", "≠"), ("\\approx", "≈"), ("\\equiv", "≡"), ("\\sim", "∼"), ("\\infty", "∞"), ("\\sum", "∑"), ("\\prod", "∏"), ("\\int", "∫"), ("\\partial", "∂"), ("\\nabla", "∇"), ("\\in", "∈"), ("\\notin", "∉"), ("\\subset", "⊂"), ("\\cup", "∪"), ("\\cap", "∩"), ("\\emptyset", "∅"), ("\\forall", "∀"), ("\\exists", "∃"), ("\\to", "→"), ("\\rightarrow", "→"), ("\\leftarrow", "←"), ("\\Rightarrow", "⇒"), ("\\Leftrightarrow", "⇔"), ("\\ldots", "…"), ("\\dots", "…"), ("\\,", " "), ("\\;", " "), ("\\quad", "  "), ("\\%", "%"), ("\\sin", "sin"), ("\\cos", "cos"), ("\\tan", "tan"), ("\\log", "log"), ("\\ln", "ln"), ("\\exp", "exp"), ("\\lim", "lim"),
];

fn script_up(c: char) -> Option<char> {
    Some(match c {
        '0' => '⁰', '1' => '¹', '2' => '²', '3' => '³', '4' => '⁴', '5' => '⁵', '6' => '⁶', '7' => '⁷', '8' => '⁸', '9' => '⁹', '+' => '⁺', '-' => '⁻', '=' => '⁼', '(' => '⁽', ')' => '⁾', 'n' => 'ⁿ', 'i' => 'ⁱ', 'x' => 'ˣ', 'y' => 'ʸ', 'a' => 'ᵃ', 'b' => 'ᵇ', 'c' => 'ᶜ', 'd' => 'ᵈ', 'e' => 'ᵉ', 'k' => 'ᵏ', 'm' => 'ᵐ', 't' => 'ᵗ',
        _ => return None,
    })
}

fn script_down(c: char) -> Option<char> {
    Some(match c {
        '0' => '₀', '1' => '₁', '2' => '₂', '3' => '₃', '4' => '₄', '5' => '₅', '6' => '₆', '7' => '₇', '8' => '₈', '9' => '₉', '+' => '₊', '-' => '₋', '=' => '₌', '(' => '₍', ')' => '₎', 'a' => 'ₐ', 'e' => 'ₑ', 'i' => 'ᵢ', 'j' => 'ⱼ', 'k' => 'ₖ', 'm' => 'ₘ', 'n' => 'ₙ', 'o' => 'ₒ', 'p' => 'ₚ', 'r' => 'ᵣ', 's' => 'ₛ', 't' => 'ₜ', 'x' => 'ₓ',
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn pie_charts_draw_a_ring_with_a_legend() {
        let svg = super::mermaid_svg("pie title Zeit <pro> Fach\n  \"Mathe\" : 42\n  \"Kunst\" : 0\n  \"Deutsch\" : 18", true).unwrap();
        assert!(svg.contains("Zeit &lt;pro&gt; Fach"));
        assert!(svg.contains(">Mathe<") && svg.contains(">Deutsch<"));
        assert!(!svg.contains(">Kunst<"));
        assert_eq!(svg.matches("<path").count(), 2);
        assert!(svg.contains("#3987E5") && !svg.contains("#2A78D6"));
    }

    #[test]
    fn a_pie_with_more_slices_than_colours_folds_the_smallest() {
        let code = format!("pie\n{}", (1..=11).map(|i| format!("  \"S{i}\" : {i}\n")).collect::<String>());
        let svg = super::mermaid_svg(&code, false).unwrap();
        assert_eq!(svg.matches("<path").count(), 8);
        assert!(svg.contains(">S11<") && svg.contains(">S5<") && !svg.contains(">S4<"));
        assert!(svg.contains(&format!(">{}<", crate::core::i18n::tr("Andere"))));
    }

    #[test]
    fn a_long_pie_title_widens_the_picture() {
        let svg = super::mermaid_svg("pie title Lernzeit pro Woche im Wintersemester nach Fach und Thema\n  \"A\" : 1\n  \"B\" : 1", false).unwrap();
        let width: f32 = svg.split("width=\"").nth(1).and_then(|w| w.split('"').next()).and_then(|w| w.parse().ok()).unwrap();
        assert!(width >= super::text_width("Lernzeit pro Woche im Wintersemester nach Fach und Thema", 15.));
        assert!(super::text_width("数学", 13.) >= 26.);
    }

    #[test]
    fn a_single_slice_pie_is_a_whole_ring() {
        let svg = super::mermaid_svg("pie\n  \"Alles\" : 3", false).unwrap();
        assert!(svg.contains("evenodd") && svg.contains(">Alles<"));
    }

    #[test]
    fn linked_mermaid_nodes_can_be_found_in_the_picture() {
        let code = "graph TD\n  A[Ziel] --> B[Anders]\n  class A internal-link\n";
        let svg = super::mermaid_svg(code, true).unwrap();
        let links = super::mermaid_links(code);
        let spots = super::mermaid_link_spots(&svg, &links);
        assert_eq!(spots.len(), 1, "{}", &svg[..svg.len().min(900)]);
        assert_eq!(spots[0].0, "Ziel");
    }

    use super::*;

    #[test]
    fn formulas_become_svg() {
        for latex in [r"e^{2i\pi} = 1", r"\frac{a}{b} + \sqrt{x}", r"\sum_{i=1}^{n} i^2", r"\alpha \leq \beta", r"\begin{pmatrix} a & b \\ c & d \end{pmatrix}", r"\text{Fläche} = \pi r^2", r"\int_0^1 x\,dx"] {
            let (svg, w, h) = math_svg(latex, true, "#222222").unwrap_or_else(|e| panic!("{latex}: {e}"));
            assert!(svg.contains("<svg"), "{latex}");
            assert!(w > 1. && h > 1., "{latex}");
        }
        assert!(math_svg(r"\frac{a", true, "#000000").is_err() || math_svg(r"\frac{a", true, "#000000").is_ok());
    }

    #[test]
    fn diagrams_become_svg() {
        assert!(mermaid_svg("flowchart LR; A-->B-->C", false).unwrap().contains("<svg"));
        assert!(mermaid_svg("sequenceDiagram\n  A->>B: Hi", true).unwrap().contains("<svg"));
    }

    #[test]
    fn diagram_nodes_can_lead_to_notes() {
        let code = "graph TD\n  Biology[Biologie] --> Chemistry\n  class Biology,Chemistry internal-link;\n";
        assert_eq!(mermaid_links(code), ["Biologie", "Chemistry"]);
    }

    #[test]
    fn simple_formulas_fit_into_text() {
        assert_eq!(math_inline(r"x^2 + y_1").as_deref(), Some("x² + y₁"));
        assert_eq!(math_inline(r"\alpha \leq \beta").as_deref(), Some("α ≤ β"));
        assert_eq!(math_inline(r"\frac{a}{b}").as_deref(), Some("(a)/(b)"));
        assert_eq!(math_inline(r"e^{2i\pi}"), None);
        assert_eq!(math_inline(r"\unbekannt{x}"), None);
    }
}

/// An SVG file as a PNG at the given scale, kept small enough for a texture.
pub fn svg_to_png(svg: &[u8], scale: f32) -> Option<Vec<u8>> {
    let tree = resvg::usvg::Tree::from_data(svg, &resvg::usvg::Options::default()).ok()?;
    let size = tree.size();
    let longest = size.width().max(size.height()) * scale;
    let scale = if longest > 8192. { scale * 8192. / longest } else { scale };
    let mut pixmap = resvg::tiny_skia::Pixmap::new((size.width() * scale).ceil() as u32, (size.height() * scale).ceil() as u32)?;
    resvg::render(&tree, resvg::tiny_skia::Transform::from_scale(scale, scale), &mut pixmap.as_mut());
    pixmap.encode_png().ok()
}

#[cfg(test)]
mod math_tests {
    #[test]
    fn every_common_latex_command_draws() {
        let failed: Vec<String> = include_str!("latex_cases.txt").lines().filter(|l| !l.is_empty()).filter_map(|line| super::math_svg(line, true, "#000000").err().map(|e| format!("{line}: {e}"))).collect();
        assert!(failed.is_empty(), "{failed:#?}");
    }

    #[test]
    fn a_broken_formula_is_an_error_not_a_crash() {
        assert!(super::math_svg(r"\frac{", true, "#000000").is_err());
        assert!(super::math_svg(r"\begin{tabular}a\end{tabular}", true, "#000000").is_err());
    }

    #[test]
    fn an_unknown_command_is_named_in_the_error() {
        assert_eq!(super::math_svg(r"\foo{x}", true, "#000000").unwrap_err(), r"Unbekannter Befehl: \foo");
        assert_eq!(super::math_svg(r"a + \fraq{1}{2}", false, "#000000").unwrap_err(), r"Unbekannter Befehl: \fraq");
        assert_eq!(super::math_markup(r"\foo{x}", false).unwrap_err(), r"Unbekannter Befehl: \foo");
        assert_eq!(super::math_svg(r"\begin{nonsense}a\end{nonsense}", true, "#000000").unwrap_err(), "Unbekannte Umgebung: nonsense");
    }

    #[test]
    fn known_commands_and_macros_are_not_flagged() {
        crate::core::vault::latex::learn_macros(r"$\newcommand{\Zwuzzel}[1]{z_{#1}}$");
        for latex in [r"\begin{pmatrix} a & b \\ c & d \end{pmatrix}", r"\begin{aligned} a &= b \\ c &= d \end{aligned}", r"\begin{cases} 1 & x > 0 \\ 0 & \text{sonst} \end{cases}", r"\mathbb{R} \mathcal{L}", r"\binom{n}{k}", r"\frac{1}{2} \sqrt{x}", r"\sum_{i=1}^n \int_0^1 x\,dx", r"\ce{H2O + CO2 -> H2CO3}", r"\Zwuzzel{3}", r"x \tag{1}"] {
            assert!(super::math_svg(latex, true, "#000000").is_ok(), "{latex}");
            assert!(super::math_markup(latex, true).is_ok(), "{latex}");
        }
    }

    #[test]
    fn a_tag_adds_its_number() {
        let plain = super::math_svg(r"x = 1", true, "#000000").unwrap();
        let tagged = super::math_svg(r"x = 1 \tag{1}", true, "#000000").unwrap();
        assert!(tagged.1 > plain.1 + 10., "{} vs {}", tagged.1, plain.1);
    }

    #[test]
    fn array_rules_are_drawn() {
        let strokes = |latex: &str| super::math_svg(latex, true, "#000000").unwrap_or_else(|e| panic!("{latex}: {e}")).0.matches("stroke=").count();
        let plain = strokes(r"\begin{array}{cc} a & b \\ c & d \end{array}");
        let barred = strokes(r"\begin{array}{c|c} a & b \\ c & d \end{array}");
        let ruled = strokes(r"\begin{array}{c|c} a & b \\ \hline c & d \end{array}");
        let boxed = strokes(r"\begin{array}{|c|c|} \hline a & b \\ \hline c & d \\ \hline \end{array}");
        assert_eq!(plain, 0);
        assert!(barred > plain && ruled > barred && boxed > ruled, "{plain} {barred} {ruled} {boxed}");
    }
}

