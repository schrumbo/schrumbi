use crate::core::pdf_export;

/// The helpers MiTeX's output calls; the rest maps onto Typst's own math.
pub const MITEX_PRELUDE: &str = r#"
#let mitexsqrt(..args) = {
  let a = args.pos()
  if a.len() == 1 { $sqrt(#a.at(0))$ } else { $root(#a.at(0), #a.at(1))$ }
}
#let mitexcolor(c, ..x) = text(fill: rgb(c), ..x)
#let textbf(it) = text(weight: "bold", it)
#let textit(it) = text(style: "italic", it)
#let textmath(it) = it
#let textmd(it) = it
#let textrm(it) = it
#let textsf(it) = it
#let texttt(it) = raw(it.text)
#let mitexmathbf(it) = math.bold(it)
#let mitexarray(arg-spec: "", ..args) = math.mat(delim: none, ..args)
#let pmatrix(..args) = math.mat(delim: "(", ..args)
#let bmatrix(..args) = math.mat(delim: "[", ..args)
#let Bmatrix(..args) = math.mat(delim: "{", ..args)
#let vmatrix(..args) = math.mat(delim: "|", ..args)
#let Vmatrix(..args) = math.mat(delim: "||", ..args)
#let matrix(..args) = math.mat(delim: none, ..args)
#let smallmatrix(..args) = math.mat(delim: none, ..args)
#let cases(..args) = math.cases(..args)
#let rcases(..args) = math.cases(reverse: true, ..args)
#let aligned(it) = it
#let align(it) = it
#let gathered(it) = it
#let split(it) = it
"#;

/// A formula as SVG in the given color, with its size in points.
pub fn math_svg(latex: &str, display: bool, color: &str) -> Result<(String, f32, f32), String> {
    let body = mitex::convert_math(latex, None)?;
    let formula = if display { format!("$ {body} $") } else { format!("${body}$") };
    let source = format!("#set page(width: auto, height: auto, margin: 1pt, fill: none)\n#set text(top-edge: \"bounds\", bottom-edge: \"bounds\")\n#set text(size: 14pt, fill: rgb(\"{color}\"))\n{MITEX_PRELUDE}\n{formula}\n");
    pdf_export::typst_svg(&source)
}

/// A formula as Typst markup, for the PDF export.
pub fn math_markup(latex: &str, display: bool) -> Result<String, String> {
    let body = mitex::convert_math(latex, None)?;
    let formula = if display { format!("$ {body} $") } else { format!("${body}$") };
    Ok(format!("#[{MITEX_PRELUDE}{formula}]"))
}

/// A Mermaid diagram as SVG.
pub fn mermaid_svg(code: &str, dark: bool) -> Result<String, String> {
    let mut theme = if dark { mermaid_rs_renderer::Theme::dark() } else { mermaid_rs_renderer::Theme::modern() };
    let (fill, border, line, text) = if dark { ("#2B3442", "#82AAF5", "#8E939B", "#EDEDED") } else { ("#E8EEF9", "#1F5FCC", "#68707C", "#1F1F1F") };
    theme.primary_color = fill.into();
    theme.primary_border_color = border.into();
    theme.primary_text_color = text.into();
    theme.line_color = line.into();
    theme.text_color = text.into();
    theme.background = "transparent".into();
    theme.edge_label_background = "transparent".into();
    let options = mermaid_rs_renderer::RenderOptions { theme, ..Default::default() };
    let caught = std::panic::catch_unwind(|| mermaid_rs_renderer::render_with_options(code, options));
    match caught {
        Ok(Ok(svg)) => Ok(svg),
        Ok(Err(e)) => Err(e.to_string()),
        Err(_) => Err("Das Diagramm ließ sich nicht zeichnen.".to_string()),
    }
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