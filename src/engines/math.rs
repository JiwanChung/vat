//! LaTeX math -> Unicode conversion for the terminal markdown view.
//!
//! Full LaTeX cannot be typeset in a terminal, so we translate the common
//! subset (Greek letters, operators, relations, super/subscripts, `\frac`,
//! `\sqrt`, accents, function names) into their closest Unicode
//! representation. Anything we do not recognise is left **verbatim** in the
//! output (e.g. an unknown `\command` stays `\command`) so the reader always
//! sees exactly what could not be converted rather than a silently mangled or
//! dropped expression.

/// Sentinel that the markdown preprocessor substitutes for backslashes inside
/// math regions, so comrak cannot strip escapes like `\,` before we parse them.
/// `latex_to_unicode` treats it exactly as a backslash.
pub const PROTECTED_BACKSLASH: char = '\u{E004}';

/// Inside a math region, markdown-active characters must be hidden from comrak
/// (otherwise `$a*b*c$` becomes emphasis, splitting the span). Each is mapped to
/// a private-use sentinel during preprocessing and restored here. Returns the
/// sentinel for a character that needs protecting, or `None` to copy it as-is.
pub fn protect_markdown_char(c: char) -> Option<char> {
    Some(match c {
        '\\' => PROTECTED_BACKSLASH,
        '*' => '\u{E005}',
        '_' => '\u{E006}',
        '`' => '\u{E007}',
        '[' => '\u{E008}',
        ']' => '\u{E009}',
        '<' => '\u{E00A}',
        '~' => '\u{E00B}',
        _ => return None,
    })
}

/// Inverse of [`protect_markdown_char`]: map a sentinel back to its character,
/// or return the character unchanged when it is not a protection sentinel.
pub fn restore_protected(c: char) -> char {
    match c {
        '\u{E004}' => '\\',
        '\u{E005}' => '*',
        '\u{E006}' => '_',
        '\u{E007}' => '`',
        '\u{E008}' => '[',
        '\u{E009}' => ']',
        '\u{E00A}' => '<',
        '\u{E00B}' => '~',
        other => other,
    }
}

/// Convert a LaTeX math expression into a Unicode approximation.
///
/// The result may contain `\n` where the source had a `\\` line break; callers
/// that render onto a single line should replace those with spaces.
pub fn latex_to_unicode(src: &str) -> String {
    let mut parser = MathParser {
        chars: src.chars().map(restore_protected).collect(),
        pos: 0,
    };
    parser.parse(false)
}

struct MathParser {
    chars: Vec<char>,
    pos: usize,
}

impl MathParser {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    /// Parse tokens, appending to a fresh string. When `stop_at_brace` is set,
    /// stop (without consuming) at the next top-level `}` so a caller reading a
    /// `{...}` group can consume the closing brace itself.
    fn parse(&mut self, stop_at_brace: bool) -> String {
        let mut out = String::new();
        while let Some(c) = self.peek() {
            match c {
                '}' if stop_at_brace => break,
                '\\' => out.push_str(&self.parse_command()),
                '^' => {
                    self.pos += 1;
                    let group = self.read_group();
                    out.push_str(&to_superscript(&group));
                }
                '_' => {
                    self.pos += 1;
                    let group = self.read_group();
                    out.push_str(&to_subscript(&group));
                }
                '{' => {
                    self.pos += 1;
                    out.push_str(&self.parse(true));
                    self.expect_close();
                }
                '~' => {
                    // Non-breaking space in LaTeX.
                    out.push(' ');
                    self.pos += 1;
                }
                _ => {
                    out.push(c);
                    self.pos += 1;
                }
            }
        }
        out
    }

    fn expect_close(&mut self) {
        if self.peek() == Some('}') {
            self.pos += 1;
        }
    }

    /// Read the argument of a `^`, `_`, `\frac`, accent, etc.: either a braced
    /// `{...}` group, a single `\command`, or a single character.
    fn read_group(&mut self) -> String {
        match self.peek() {
            Some('{') => {
                self.pos += 1;
                let inner = self.parse(true);
                self.expect_close();
                inner
            }
            Some('\\') => self.parse_command(),
            Some(c) => {
                self.pos += 1;
                c.to_string()
            }
            None => String::new(),
        }
    }

    /// Read an optional `[...]` argument (e.g. the index of `\sqrt[3]{x}`).
    fn read_optional_bracket(&mut self) -> Option<String> {
        if self.peek() != Some('[') {
            return None;
        }
        self.pos += 1;
        let mut inner = String::new();
        while let Some(c) = self.peek() {
            self.pos += 1;
            if c == ']' {
                return Some(inner);
            }
            inner.push(c);
        }
        Some(inner)
    }

    /// Handle a backslash-introduced token. `self.pos` points at the `\`.
    fn parse_command(&mut self) -> String {
        self.pos += 1; // consume the backslash
        let c = match self.peek() {
            Some(c) => c,
            None => return "\\".to_string(),
        };

        if !c.is_ascii_alphabetic() {
            self.pos += 1;
            return match c {
                '\\' => "\n".to_string(),
                ' ' | ',' | ';' | ':' => " ".to_string(),
                '!' => String::new(), // negative thin space
                '{' | '}' | '%' | '$' | '#' | '&' | '_' => c.to_string(),
                _ => format!("\\{}", c),
            };
        }

        let start = self.pos;
        while matches!(self.peek(), Some(ch) if ch.is_ascii_alphabetic()) {
            self.pos += 1;
        }
        let name: String = self.chars[start..self.pos].iter().collect();

        match name.as_str() {
            "frac" | "dfrac" | "tfrac" => {
                let num = self.read_group();
                let den = self.read_group();
                format!("{}/{}", paren_if_compound(&num), paren_if_compound(&den))
            }
            "sqrt" => {
                let index = self.read_optional_bracket();
                let radicand = self.read_group();
                match index {
                    Some(idx) if !idx.is_empty() => {
                        format!("{}√{}", to_superscript(&idx), paren_if_compound(&radicand))
                    }
                    _ => format!("√{}", paren_if_compound(&radicand)),
                }
            }
            // Blackboard-bold: map the well-known number sets to their glyphs.
            "mathbb" => {
                let inner = self.read_group();
                blackboard_bold(&inner).unwrap_or(inner)
            }
            // Font/style wrappers: keep the content, drop the styling.
            "text" | "textrm" | "textbf" | "textit" | "mathrm" | "mathbf" | "mathit"
            | "mathsf" | "mathtt" | "mathcal" | "mathfrak" | "boldsymbol" | "operatorname"
            | "mbox" => self.read_group(),
            // Accents: attach a Unicode combining mark to the argument.
            "hat" | "widehat" => combine(self.read_group(), '\u{0302}'),
            "bar" | "overline" => combine(self.read_group(), '\u{0304}'),
            "vec" => combine(self.read_group(), '\u{20D7}'),
            "tilde" | "widetilde" => combine(self.read_group(), '\u{0303}'),
            "dot" => combine(self.read_group(), '\u{0307}'),
            "ddot" => combine(self.read_group(), '\u{0308}'),
            // Sizing / styling directives with no glyph of their own.
            "left" | "right" | "big" | "bigl" | "bigr" | "Big" | "Bigl" | "Bigr" | "bigg"
            | "biggl" | "biggr" | "Bigg" | "displaystyle" | "textstyle" | "scriptstyle"
            | "limits" | "nolimits" | "quad" | "qquad" => {
                if name == "quad" || name == "qquad" {
                    "  ".to_string()
                } else {
                    String::new()
                }
            }
            other => {
                if let Some(sym) = symbol(other) {
                    sym.to_string()
                } else if is_function_name(other) {
                    other.to_string()
                } else {
                    // Unknown command: leave it verbatim so nothing is silently lost.
                    format!("\\{}", other)
                }
            }
        }
    }
}

/// Wrap a rendered fragment in parentheses when it is more than one grapheme,
/// so `\frac{a+b}{c}` reads as `(a+b)/c` rather than `a+b/c`.
fn paren_if_compound(s: &str) -> String {
    let trimmed = s.trim();
    if trimmed.chars().count() <= 1 || is_parenthesised(trimmed) {
        trimmed.to_string()
    } else {
        format!("({})", trimmed)
    }
}

fn is_parenthesised(s: &str) -> bool {
    (s.starts_with('(') && s.ends_with(')'))
        || (s.starts_with('[') && s.ends_with(']'))
        || (s.starts_with('|') && s.ends_with('|'))
}

/// Apply a combining mark to the first character of `s` (LaTeX accents apply to
/// a single base symbol; multi-character arguments are uncommon).
fn combine(s: String, mark: char) -> String {
    let mut out = String::new();
    let mut chars = s.chars();
    if let Some(first) = chars.next() {
        out.push(first);
        out.push(mark);
    }
    out.extend(chars);
    out
}

fn to_superscript(s: &str) -> String {
    render_script(s, superscript_char, '^')
}

fn to_subscript(s: &str) -> String {
    render_script(s, subscript_char, '_')
}

/// Map every character of a super/subscript group through `map`. If any
/// character has no Unicode form, fall back to a visible `^(...)` / `_(...)`
/// notation rather than dropping or approximating it.
fn render_script(s: &str, map: fn(char) -> Option<char>, marker: char) -> String {
    let mut out = String::new();
    for c in s.chars() {
        match map(c) {
            Some(m) => out.push(m),
            None => {
                if s.chars().count() == 1 {
                    return format!("{}{}", marker, s);
                }
                return format!("{}({})", marker, s);
            }
        }
    }
    out
}

fn superscript_char(c: char) -> Option<char> {
    Some(match c {
        '0' => '\u{2070}',
        '1' => '\u{00B9}',
        '2' => '\u{00B2}',
        '3' => '\u{00B3}',
        '4' => '\u{2074}',
        '5' => '\u{2075}',
        '6' => '\u{2076}',
        '7' => '\u{2077}',
        '8' => '\u{2078}',
        '9' => '\u{2079}',
        '+' => '\u{207A}',
        '-' => '\u{207B}',
        '=' => '\u{207C}',
        '(' => '\u{207D}',
        ')' => '\u{207E}',
        'a' => 'ᵃ',
        'b' => 'ᵇ',
        'c' => 'ᶜ',
        'd' => 'ᵈ',
        'e' => 'ᵉ',
        'f' => 'ᶠ',
        'g' => 'ᵍ',
        'h' => 'ʰ',
        'i' => 'ⁱ',
        'j' => 'ʲ',
        'k' => 'ᵏ',
        'l' => 'ˡ',
        'm' => 'ᵐ',
        'n' => 'ⁿ',
        'o' => 'ᵒ',
        'p' => 'ᵖ',
        'r' => 'ʳ',
        's' => 'ˢ',
        't' => 'ᵗ',
        'u' => 'ᵘ',
        'v' => 'ᵛ',
        'w' => 'ʷ',
        'x' => 'ˣ',
        'y' => 'ʸ',
        'z' => 'ᶻ',
        ' ' => ' ',
        _ => return None,
    })
}

fn subscript_char(c: char) -> Option<char> {
    Some(match c {
        '0' => '\u{2080}',
        '1' => '\u{2081}',
        '2' => '\u{2082}',
        '3' => '\u{2083}',
        '4' => '\u{2084}',
        '5' => '\u{2085}',
        '6' => '\u{2086}',
        '7' => '\u{2087}',
        '8' => '\u{2088}',
        '9' => '\u{2089}',
        '+' => '\u{208A}',
        '-' => '\u{208B}',
        '=' => '\u{208C}',
        '(' => '\u{208D}',
        ')' => '\u{208E}',
        'a' => 'ₐ',
        'e' => 'ₑ',
        'h' => 'ₕ',
        'i' => 'ᵢ',
        'j' => 'ⱼ',
        'k' => 'ₖ',
        'l' => 'ₗ',
        'm' => 'ₘ',
        'n' => 'ₙ',
        'o' => 'ₒ',
        'p' => 'ₚ',
        'r' => 'ᵣ',
        's' => 'ₛ',
        't' => 'ₜ',
        'u' => 'ᵤ',
        'v' => 'ᵥ',
        'x' => 'ₓ',
        ' ' => ' ',
        _ => return None,
    })
}

/// Map the common blackboard-bold number sets to their Unicode glyphs. Returns
/// `None` for letters without a well-known glyph so the caller can keep the
/// plain letter rather than substitute something surprising.
fn blackboard_bold(s: &str) -> Option<String> {
    let c = match s.trim() {
        "R" => 'ℝ',
        "N" => 'ℕ',
        "Z" => 'ℤ',
        "Q" => 'ℚ',
        "C" => 'ℂ',
        "H" => 'ℍ',
        "P" => 'ℙ',
        _ => return None,
    };
    Some(c.to_string())
}

fn is_function_name(name: &str) -> bool {
    matches!(
        name,
        "sin" | "cos" | "tan" | "cot" | "sec" | "csc" | "sinh" | "cosh" | "tanh" | "coth"
            | "arcsin" | "arccos" | "arctan" | "log" | "ln" | "lg" | "exp" | "lim" | "limsup"
            | "liminf" | "max" | "min" | "sup" | "inf" | "det" | "gcd" | "deg" | "arg" | "dim"
            | "ker" | "hom" | "mod" | "Pr"
    )
}

/// Map a LaTeX symbol command name (without the backslash) to Unicode.
fn symbol(name: &str) -> Option<&'static str> {
    Some(match name {
        // Lowercase Greek.
        "alpha" => "α",
        "beta" => "β",
        "gamma" => "γ",
        "delta" => "δ",
        "epsilon" => "ϵ",
        "varepsilon" => "ε",
        "zeta" => "ζ",
        "eta" => "η",
        "theta" => "θ",
        "vartheta" => "ϑ",
        "iota" => "ι",
        "kappa" => "κ",
        "lambda" => "λ",
        "mu" => "μ",
        "nu" => "ν",
        "xi" => "ξ",
        "omicron" => "ο",
        "pi" => "π",
        "varpi" => "ϖ",
        "rho" => "ρ",
        "varrho" => "ϱ",
        "sigma" => "σ",
        "varsigma" => "ς",
        "tau" => "τ",
        "upsilon" => "υ",
        "phi" => "φ",
        "varphi" => "ϕ",
        "chi" => "χ",
        "psi" => "ψ",
        "omega" => "ω",
        // Uppercase Greek.
        "Gamma" => "Γ",
        "Delta" => "Δ",
        "Theta" => "Θ",
        "Lambda" => "Λ",
        "Xi" => "Ξ",
        "Pi" => "Π",
        "Sigma" => "Σ",
        "Upsilon" => "Υ",
        "Phi" => "Φ",
        "Psi" => "Ψ",
        "Omega" => "Ω",
        // Binary operators.
        "times" => "×",
        "div" => "÷",
        "pm" => "±",
        "mp" => "∓",
        "cdot" => "⋅",
        "ast" => "∗",
        "star" => "⋆",
        "circ" => "∘",
        "bullet" => "∙",
        "oplus" => "⊕",
        "ominus" => "⊖",
        "otimes" => "⊗",
        "oslash" => "⊘",
        "odot" => "⊙",
        "setminus" => "∖",
        "cup" => "∪",
        "cap" => "∩",
        "sqcup" => "⊔",
        "sqcap" => "⊓",
        "wedge" | "land" => "∧",
        "vee" | "lor" => "∨",
        "amalg" => "⨿",
        "uplus" => "⊎",
        // Relations.
        "leq" | "le" => "≤",
        "geq" | "ge" => "≥",
        "neq" | "ne" => "≠",
        "equiv" => "≡",
        "approx" => "≈",
        "cong" => "≅",
        "simeq" => "≃",
        "sim" => "∼",
        "propto" => "∝",
        "ll" => "≪",
        "gg" => "≫",
        "prec" => "≺",
        "succ" => "≻",
        "preceq" => "⪯",
        "succeq" => "⪰",
        "subset" => "⊂",
        "supset" => "⊃",
        "subseteq" => "⊆",
        "supseteq" => "⊇",
        "sqsubseteq" => "⊑",
        "sqsupseteq" => "⊒",
        "in" => "∈",
        "notin" => "∉",
        "ni" => "∋",
        "mid" => "∣",
        "nmid" => "∤",
        "parallel" => "∥",
        "perp" => "⊥",
        "models" => "⊨",
        "vdash" => "⊢",
        "dashv" => "⊣",
        "doteq" => "≐",
        "asymp" => "≍",
        "bowtie" => "⋈",
        // Logic / set.
        "forall" => "∀",
        "exists" => "∃",
        "nexists" => "∄",
        "neg" | "lnot" => "¬",
        "top" => "⊤",
        "bot" => "⊥",
        "emptyset" | "varnothing" => "∅",
        "infty" => "∞",
        "aleph" => "ℵ",
        "complement" => "∁",
        "therefore" => "∴",
        "because" => "∵",
        // Arrows.
        "rightarrow" | "to" => "→",
        "leftarrow" | "gets" => "←",
        "leftrightarrow" => "↔",
        "Rightarrow" | "implies" => "⇒",
        "Leftarrow" => "⇐",
        "Leftrightarrow" | "iff" => "⇔",
        "mapsto" => "↦",
        "longrightarrow" => "⟶",
        "longleftarrow" => "⟵",
        "uparrow" => "↑",
        "downarrow" => "↓",
        "updownarrow" => "↕",
        "nearrow" => "↗",
        "searrow" => "↘",
        "swarrow" => "↙",
        "nwarrow" => "↖",
        "hookrightarrow" => "↪",
        "hookleftarrow" => "↩",
        // Big operators.
        "sum" => "∑",
        "prod" => "∏",
        "coprod" => "∐",
        "int" => "∫",
        "iint" => "∬",
        "iiint" => "∭",
        "oint" => "∮",
        "bigcup" => "⋃",
        "bigcap" => "⋂",
        "bigvee" => "⋁",
        "bigwedge" => "⋀",
        "bigoplus" => "⨁",
        "bigotimes" => "⨂",
        "bigodot" => "⨀",
        "bigsqcup" => "⨆",
        // Calculus / misc symbols.
        "partial" => "∂",
        "nabla" => "∇",
        "hbar" => "ℏ",
        "ell" => "ℓ",
        "Re" => "ℜ",
        "Im" => "ℑ",
        "wp" => "℘",
        "prime" => "′",
        "dagger" => "†",
        "ddagger" => "‡",
        "angle" => "∠",
        "measuredangle" => "∡",
        "triangle" => "△",
        "square" => "□",
        "blacksquare" => "■",
        "diamond" => "⋄",
        "lozenge" => "◊",
        "flat" => "♭",
        "sharp" => "♯",
        "natural" => "♮",
        "clubsuit" => "♣",
        "diamondsuit" => "♦",
        "heartsuit" => "♥",
        "spadesuit" => "♠",
        "checkmark" => "✓",
        "surd" => "√",
        // Dots and ellipses.
        "ldots" | "dots" => "…",
        "cdots" => "⋯",
        "vdots" => "⋮",
        "ddots" => "⋱",
        // Delimiters.
        "langle" => "⟨",
        "rangle" => "⟩",
        "lfloor" => "⌊",
        "rfloor" => "⌋",
        "lceil" => "⌈",
        "rceil" => "⌉",
        "backslash" => "\\",
        // Named spaces / misc.
        "degree" => "°",
        "cdotp" => "·",
        "colon" => ":",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn greek_and_operators() {
        assert_eq!(latex_to_unicode("\\alpha + \\beta"), "α + β");
        assert_eq!(latex_to_unicode("a \\times b"), "a × b");
        assert_eq!(latex_to_unicode("x \\leq y"), "x ≤ y");
    }

    #[test]
    fn superscripts_and_subscripts() {
        assert_eq!(latex_to_unicode("x^2"), "x²");
        assert_eq!(latex_to_unicode("x^{10}"), "x¹⁰");
        assert_eq!(latex_to_unicode("x_i"), "xᵢ");
        assert_eq!(latex_to_unicode("a_{ij}"), "aᵢⱼ");
        assert_eq!(latex_to_unicode("e^{-x}"), "e⁻ˣ");
    }

    #[test]
    fn fraction_and_sqrt() {
        assert_eq!(latex_to_unicode("\\frac{1}{2}"), "1/2");
        assert_eq!(latex_to_unicode("\\frac{a+b}{c}"), "(a+b)/c");
        assert_eq!(latex_to_unicode("\\sqrt{2}"), "√2");
        assert_eq!(latex_to_unicode("\\sqrt{x+1}"), "√(x+1)");
    }

    #[test]
    fn famous_identities() {
        assert_eq!(latex_to_unicode("E = mc^2"), "E = mc²");
        assert_eq!(latex_to_unicode("a^2 + b^2 = c^2"), "a² + b² = c²");
        assert_eq!(latex_to_unicode("\\sum_{i=1}^{n} i"), "∑ᵢ₌₁ⁿ i");
    }

    #[test]
    fn unknown_command_is_preserved_verbatim() {
        // Nothing is silently dropped: an unrecognised command stays visible.
        assert_eq!(latex_to_unicode("\\foobar x"), "\\foobar x");
        assert_eq!(latex_to_unicode("x^{\\zeta}"), "x^ζ");
    }

    #[test]
    fn blackboard_bold_sets() {
        assert_eq!(latex_to_unicode("x \\in \\mathbb{R}"), "x ∈ ℝ");
        // A letter with no well-known glyph keeps its plain form.
        assert_eq!(latex_to_unicode("\\mathbb{K}"), "K");
    }

    #[test]
    fn functions_lose_the_backslash() {
        assert_eq!(latex_to_unicode("\\sin x"), "sin x");
        assert_eq!(latex_to_unicode("\\log_2 n"), "log₂ n");
    }
}

