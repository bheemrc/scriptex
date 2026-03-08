use crate::color::Color;

/// The entire parsed document
#[derive(Debug, Clone)]
pub struct Document {
    pub class: DocumentClass,
    pub preamble: Preamble,
    pub body: Vec<Node>,
}

#[derive(Debug, Clone)]
pub struct DocumentClass {
    pub class_type: ClassType,
    pub options: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ClassType {
    Article,
    Report,
    Book,
    Letter,
    Beamer,
    Memoir,
    Custom(String),
}

#[derive(Debug, Clone)]
pub struct Preamble {
    pub title: Option<String>,
    pub author: Option<String>,
    pub date: Option<String>,
    pub packages: Vec<Package>,
    pub page_setup: PageSetup,
    pub font_size: f32,
    pub line_spacing: f32,
    pub paragraph_indent: Option<f32>,  // \parindent
    pub paragraph_skip: Option<f32>,    // \parskip
    pub commands: Vec<(String, Vec<Node>)>,
    pub theorem_defs: Vec<TheoremDef>,
    pub page_style: String, // "plain", "headings", "empty", "fancy"
    pub fancy_header: FancyHeaderFooter,
    pub addresses: Vec<AuthorAddress>,
    pub keywords: Option<String>,
    pub subjclass: Option<(String, String)>, // (year, classification text)
    pub hyperref: HyperrefConfig,
    pub array_stretch: f32,  // \arraystretch (default 1.0)
}

/// hyperref package configuration
#[derive(Debug, Clone)]
pub struct HyperrefConfig {
    pub color_links: bool,
    pub link_color: Option<String>,  // internal links color name
    pub url_color: Option<String>,   // URL link color name
    pub cite_color: Option<String>,  // citation link color name
}

impl Default for HyperrefConfig {
    fn default() -> Self {
        Self { color_links: true, link_color: None, url_color: None, cite_color: None }
    }
}

/// fancyhdr header/footer configuration
#[derive(Debug, Clone, Default)]
pub struct FancyHeaderFooter {
    pub head_left: String,
    pub head_center: String,
    pub head_right: String,
    pub foot_left: String,
    pub foot_center: String,
    pub foot_right: String,
    pub head_rule_width: f32, // 0.4pt default, 0 = no rule
    pub foot_rule_width: f32, // 0pt default
}

#[derive(Debug, Clone)]
pub struct AuthorAddress {
    pub address: String,
    pub email: Option<String>,
}

impl Default for Preamble {
    fn default() -> Self {
        Preamble {
            title: None,
            author: None,
            date: None,
            packages: Vec::new(),
            page_setup: PageSetup::default(),
            font_size: 10.0,
            line_spacing: 1.0,
            paragraph_indent: None,
            paragraph_skip: None,
            commands: Vec::new(),
            theorem_defs: Vec::new(),
            page_style: String::new(), // empty = default (plain for article)
            fancy_header: FancyHeaderFooter::default(),
            addresses: Vec::new(),
            keywords: None,
            subjclass: None,
            hyperref: HyperrefConfig::default(),
            array_stretch: 1.0,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Package {
    pub name: String,
    pub options: Vec<String>,
}

#[derive(Debug, Clone, Copy)]
pub struct PageSetup {
    pub width: f32,       // points
    pub height: f32,      // points
    pub margin_top: f32,
    pub margin_bottom: f32,
    pub margin_left: f32,
    pub margin_right: f32,
    pub header_height: f32,
    pub footer_height: f32,
    pub columns: u32,
    pub column_sep: f32,
}

impl Default for PageSetup {
    fn default() -> Self {
        // LaTeX article class defaults for US Letter (10pt):
        // textwidth = 345pt, textheight ≈ 598pt
        // Horizontal: 1in hoffset + oddsidemargin(0.39in) ≈ 133.5pt each side
        // Vertical: 1in voffset + topmargin(0) + headheight(12) + headsep(25) ≈ 97pt top
        PageSetup {
            width: 612.0,
            height: 792.0,
            margin_top: 89.0,    // ~1.24in (LaTeX: 1in + topmargin + headheight + headsep)
            margin_bottom: 105.0, // to give textheight ≈ 598pt
            margin_left: 133.5,  // (612 - 345) / 2 ≈ 133.5pt for centered text
            margin_right: 133.5,
            header_height: 0.0,
            footer_height: 30.0,  // LaTeX \footskip = 30pt
            columns: 1,
            column_sep: 10.0,    // LaTeX \columnsep = 10pt
        }
    }
}

impl PageSetup {
    pub fn a4() -> Self {
        PageSetup {
            width: 595.276,
            height: 841.890,
            ..Default::default()
        }
    }

    pub fn text_width(&self) -> f32 {
        self.width - self.margin_left - self.margin_right
    }

    pub fn text_height(&self) -> f32 {
        self.height - self.margin_top - self.margin_bottom - self.header_height - self.footer_height
    }

    pub fn column_width(&self) -> f32 {
        if self.columns <= 1 {
            self.text_width()
        } else {
            (self.text_width() - (self.columns as f32 - 1.0) * self.column_sep) / self.columns as f32
        }
    }
}

/// Core AST node types
#[derive(Debug, Clone)]
pub enum Node {
    /// Raw text content
    Text(String),
    /// Text as source reference (offset, length) - avoids allocation
    TextRef(u32, u32),

    /// A paragraph (sequence of inline content)
    Paragraph(Vec<Node>),
    /// Optimized: single-text paragraph as source reference (offset, length)
    TextParagraph(u32, u32),

    /// Line break
    LineBreak,

    /// Page break
    PageBreak,

    /// Switch to two-column mode, with optional spanning content
    TwoColumn(Vec<Node>),

    /// Switch to one-column mode
    OneColumn,

    /// Horizontal space
    HSpace(f32),

    /// Vertical space
    VSpace(f32),
    /// Set paragraph indent
    SetParIndent(f32),
    /// Set paragraph skip (\parskip)
    SetParSkip(f32),
    /// Set baseline skip (\baselineskip)
    SetBaselineSkip(f32),
    /// Alignment declaration (\centering, \raggedright, \raggedleft)
    AlignmentDecl(AlignmentMode),

    /// Section heading
    Section {
        level: SectionLevel,
        title: Vec<Node>,
        numbered: bool,
    },

    /// Font style changes
    Bold(Vec<Node>),
    Italic(Vec<Node>),
    Monospace(Vec<Node>),
    SmallCaps(Vec<Node>),
    SansSerif(Vec<Node>),
    Underline(Vec<Node>),
    Strikethrough(Vec<Node>),
    Superscript(Vec<Node>),
    Subscript(Vec<Node>),

    /// Font size change
    FontSize {
        size: FontSizeSpec,
        content: Vec<Node>,
    },

    /// Color
    Colored {
        color: Color,
        content: Vec<Node>,
    },

    /// Emphasis (toggles italic)
    Emph(Vec<Node>),

    /// Math mode
    InlineMath(Vec<MathNode>),
    DisplayMath(Box<DisplayMathData>),

    /// Environments
    Environment(Box<EnvironmentData>),

    /// Lists
    ItemizeList(Vec<ListItem>),
    EnumerateList(Vec<ListItem>),
    DescriptionList(Vec<ListItem>),

    /// Table
    Table(Box<Table>),

    /// Figure
    Figure(Box<FigureData>),

    /// Image
    Image(Box<ImageData>),

    /// Title page elements
    MakeTitle,

    /// Table of contents
    TableOfContents,
    ListOfFigures,
    ListOfTables,

    /// Page numbering style change: "arabic", "roman", "Roman", "alph", "Alph"
    PageNumbering(String),

    /// Switch to appendix mode (sections numbered A, B, C...)
    Appendix,

    /// Footnote
    Footnote(Vec<Node>),

    /// Hyperlink
    Href { url: String, content: Vec<Node> },

    /// Cross-reference
    Label(String),
    Ref(String),
    /// Clever reference — renders as "Type N" (\cref) or "TYPE N" (\Cref)
    Cref(String, bool),
    /// Equation reference — renders as (N)
    EqRef(String),
    /// Citation — key, optional argument text (e.g. "Prop.~1.6"), citation style
    Citation(String, Option<String>, CitationStyle),
    /// Bibliography item marker
    BibItem(String),

    /// Horizontal rule
    HRule,
    /// Inline rule with dimensions: \rule[raise]{width}{height}
    Rule { width: f32, height: f32 },

    /// Quote/quotation environment
    Quote(Vec<Node>),
    Quotation(Vec<Node>),

    /// Verbatim/code
    Verbatim(String),
    Code(String),

    /// Abstract
    Abstract(Vec<Node>),

    /// Centering group
    Center(Vec<Node>),
    FlushLeft(Vec<Node>),
    FlushRight(Vec<Node>),

    /// Minipage
    Minipage {
        width: f32,
        content: Vec<Node>,
    },

    /// Theorem-like environments
    Theorem(Box<TheoremData>),
    /// Proof environment with optional header (e.g. "Proof of Theorem 1.2")
    Proof { header: Option<String>, content: Vec<Node> },
    /// Expandable horizontal fill
    HFill,

    /// Font style declaration (e.g. \bfseries, \itshape) — changes style for subsequent siblings
    FontStyleDecl(FontDeclType),

    /// Color declaration (e.g. \color{red}) — changes color for subsequent siblings
    ColorDecl(Color),

    /// Raw content (pass-through)
    Raw(String),

    /// Group (braces)
    Group(Vec<Node>),

    /// Suppress indent on next paragraph
    NoIndent,

    /// Set a counter value
    SetCounter(String, i32),

    /// Define a custom color (\definecolor)
    DefineColor { name: String, color: Color },

    /// ZapfDingbats character (byte code in ZapfDingbats encoding)
    Dingbat(u8),

    /// Typeset logos
    LaTeXLogo,
    TeXLogo,

    /// Special characters
    NonBreakingSpace,
    EnDash,
    EmDash,
    LeftQuote,
    RightQuote,
    LeftDoubleQuote,
    RightDoubleQuote,
    Ellipsis,
    Copyright,
    Registered,
    Trademark,
    Ampersand,
    Percent,
    Dollar,
    Hash,
    Underscore,
    Backslash,
    Tilde,
    Caret,
    LeftBrace,
    RightBrace,

    /// Colored/framed box (tcolorbox, mdframed, etc.)
    ColorBox(Box<ColorBoxData>),

    /// Wrapped figure (text flows around it)
    WrapFigure {
        placement: char,      // 'r' or 'l'
        width: f32,           // width as fraction of text width
        content: Vec<Node>,
        caption: Option<Vec<Node>>,
        label: Option<String>,
    },

    /// Sub-figure within a figure environment
    SubFigure {
        width: f32,
        content: Vec<Node>,
        caption: Option<Vec<Node>>,
    },

    /// Algorithm float (like figure/table with caption/label)
    Algorithm {
        caption: Option<String>,
        label: Option<String>,
        content: Vec<AlgoLine>,
        line_numbered: bool,
    },
}

/// A line in an algorithmic/pseudocode environment
#[derive(Debug, Clone)]
pub struct AlgoLine {
    pub indent: u32,
    pub content: Vec<AlgoToken>,
}

/// Token in an algorithm line
#[derive(Debug, Clone)]
pub enum AlgoToken {
    Keyword(String),
    Text(String),
    Math(Vec<MathNode>),
}

#[derive(Debug, Clone)]
pub struct ColorBoxData {
    pub content: Vec<Node>,
    pub title: Option<Vec<Node>>,
    pub bg_color: Color,
    pub frame_color: Color,
    pub corner_radius: f32,  // in points
    pub rule_width: f32,     // frame thickness in points
    pub padding: f32,        // inner padding in points
}

#[derive(Debug, Clone)]
pub struct TheoremData {
    pub env_name: String,
    pub title: String,          // e.g. "Theorem", "Lemma", "Definition"
    pub number: Option<u32>,
    pub optional_name: Option<String>, // e.g. [Zorn's Lemma]
    pub body: Vec<Node>,
    pub italic_body: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TheoremStyle {
    Plain,      // bold label, italic body (default)
    Definition, // bold label, upright body
    Remark,     // italic label, upright body
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CitationStyle {
    /// \cite{} or \citep{} — [Author et al., 2024] or (Author et al., 2024)
    Numeric,
    /// \citep{} — (Author et al., 2024)
    Parenthetical,
    /// \citet{} — Author et al. (2024)
    Textual,
    /// \citeauthor{} — Author et al.
    AuthorOnly,
    /// \citeyear{} — 2024
    YearOnly,
    /// \citealt{} / \citealp{} — Author et al. 2024 (no parens)
    AltNoParen,
}

#[derive(Debug, Clone)]
pub struct TheoremDef {
    pub env_name: String,
    pub display_title: String,
    pub numbered: bool,
    pub counter: Option<String>, // shared counter name, or None for own counter
    pub style: TheoremStyle,
}

#[derive(Debug, Clone)]
pub struct DisplayMathData {
    pub nodes: Vec<MathNode>,
    pub numbered: bool,
    pub env_type: MathEnvType,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MathEnvType {
    DollarDollar,
    Equation,
    Align,
    Gather,
    Multline,
}

#[derive(Debug, Clone)]
pub struct EnvironmentData {
    pub name: String,
    pub args: Vec<String>,
    pub content: Vec<Node>,
}

#[derive(Debug, Clone)]
pub struct FigureData {
    pub content: Vec<Node>,
    pub caption: Option<Vec<Node>>,
    pub label: Option<String>,
    pub placement: String,
    pub starred: bool,
}

#[derive(Debug, Clone)]
pub struct ImageData {
    pub path: String,
    pub width: Option<f32>,
    pub height: Option<f32>,
    pub scale: Option<f32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SectionLevel {
    Part,
    Chapter,
    Section,
    Subsection,
    Subsubsection,
    Paragraph,
    Subparagraph,
}

impl SectionLevel {
    pub fn depth(&self) -> i32 {
        match self {
            SectionLevel::Part => -1,
            SectionLevel::Chapter => 0,
            SectionLevel::Section => 1,
            SectionLevel::Subsection => 2,
            SectionLevel::Subsubsection => 3,
            SectionLevel::Paragraph => 4,
            SectionLevel::Subparagraph => 5,
        }
    }

    pub fn font_size(&self, base: f32) -> f32 {
        match self {
            SectionLevel::Part => base * 2.0,
            SectionLevel::Chapter => base * 1.728,
            SectionLevel::Section => base * 1.44,
            SectionLevel::Subsection => base * 1.2,
            SectionLevel::Subsubsection => base,
            SectionLevel::Paragraph => base,
            SectionLevel::Subparagraph => base,
        }
    }

    pub fn spacing_before(&self) -> f32 {
        match self {
            SectionLevel::Part => 36.0,
            SectionLevel::Chapter => 30.0,
            SectionLevel::Section => 18.0,   // LaTeX: 3.5ex ≈ 15pt + glue
            SectionLevel::Subsection => 14.0, // LaTeX: 3.25ex ≈ 14pt
            SectionLevel::Subsubsection => 13.5, // LaTeX: 3.25ex ≈ 13.5pt
            SectionLevel::Paragraph => 10.0,
            SectionLevel::Subparagraph => 8.0,
        }
    }

    pub fn spacing_after(&self) -> f32 {
        match self {
            SectionLevel::Part => 20.0,
            SectionLevel::Chapter => 16.0,
            SectionLevel::Section => 10.0,
            SectionLevel::Subsection => 8.0,
            SectionLevel::Subsubsection => 6.0,
            SectionLevel::Paragraph => 4.0,
            SectionLevel::Subparagraph => 4.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FontDeclType {
    Bold,
    Italic,
    Monospace,
    Regular,
    SmallCaps,
    SansSerif,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AlignmentMode {
    Justify,
    Center,
    FlushLeft,  // \raggedright
    FlushRight, // \raggedleft
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FontSizeSpec {
    Tiny,
    Scriptsize,
    Footnotesize,
    Small,
    Normalsize,
    Large,
    LargeX,
    LargeXX,
    Huge,
    HugeX,
    Points(f32),
}

impl FontSizeSpec {
    pub fn to_points(&self, base: f32) -> f32 {
        match self {
            FontSizeSpec::Tiny => base * 0.5,
            FontSizeSpec::Scriptsize => base * 0.6,
            FontSizeSpec::Footnotesize => base * 0.7,
            FontSizeSpec::Small => base * 0.85,
            FontSizeSpec::Normalsize => base,
            FontSizeSpec::Large => base * 1.2,
            FontSizeSpec::LargeX => base * 1.44,
            FontSizeSpec::LargeXX => base * 1.728,
            FontSizeSpec::Huge => base * 2.074,
            FontSizeSpec::HugeX => base * 2.488,
            FontSizeSpec::Points(p) => *p,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ListItem {
    pub label: Option<Vec<Node>>,
    pub content: Vec<Node>,
}

#[derive(Debug, Clone)]
pub struct Table {
    pub columns: Vec<ColumnSpec>,
    pub rows: Vec<TableRow>,
    pub caption: Option<Vec<Node>>,
    pub label: Option<String>,
    pub centering: bool,
}

#[derive(Debug, Clone)]
pub enum ColumnSpec {
    Left,
    Center,
    Right,
    Paragraph(f32),
    Separator,
}

#[derive(Debug, Clone)]
pub struct TableRow {
    pub cells: Vec<TableCell>,
    pub hline_before: bool,
    pub hline_after: bool,
    pub extra_space_before: f32,
    /// Partial column rules: (start_col, end_col) — 1-based as in LaTeX
    pub cmidrules: Vec<(u32, u32)>,
}

#[derive(Debug, Clone)]
pub struct TableCell {
    pub content: Vec<Node>,
    pub colspan: u32,
    pub rowspan: u32,
    pub alignment: Option<ColumnSpec>,
}

/// Math AST nodes
#[derive(Debug, Clone)]
pub enum MathNode {
    Number(String),
    Variable(char),
    Operator(String),
    Text(String),
    Frac { numer: Vec<MathNode>, denom: Vec<MathNode> },
    Sqrt { index: Option<Vec<MathNode>>, radicand: Vec<MathNode> },
    Super(Vec<MathNode>),
    Sub(Vec<MathNode>),
    Group(Vec<MathNode>),
    Function(String),
    Symbol(String),
    Space(f32),
    Left(String),
    Right(String),
    DelimitedGroup { left: String, right: String, content: Vec<MathNode> },
    Sum { lower: Option<Vec<MathNode>>, upper: Option<Vec<MathNode>> },
    Integral { lower: Option<Vec<MathNode>>, upper: Option<Vec<MathNode>> },
    Product { lower: Option<Vec<MathNode>>, upper: Option<Vec<MathNode>> },
    Matrix { rows: Vec<Vec<Vec<MathNode>>>, style: MatrixStyle },
    Cases { rows: Vec<(Vec<MathNode>, Option<Vec<MathNode>>)> },
    Accent { base: Vec<MathNode>, accent_type: AccentType },
    Over { content: Vec<MathNode>, over_type: OverType },
    Under { content: Vec<MathNode>, under_type: UnderType },
    Binom { top: Vec<MathNode>, bottom: Vec<MathNode> },
    Overset { over: Vec<MathNode>, base: Vec<MathNode> },
    Underset { under: Vec<MathNode>, base: Vec<MathNode> },
    OperatorName(String),
    MathFont { font: MathFontType, content: Vec<MathNode> },
    AlignmentMark,
    NewLine,
    Phantom(Vec<MathNode>),
    StyleSwitch(MathStyleType),
    BigDelim { delim: String, size: f32 },
    /// Boxed expression (thin frame around content)
    Boxed(Vec<MathNode>),
    /// Limit-style operator (lim, sup, inf, max, min) with limits below/above
    LimitOp { name: String, lower: Option<Vec<MathNode>>, upper: Option<Vec<MathNode>> },
    /// Suppress equation number on this row
    NoTag,
    /// Custom equation tag
    Tag(String),
    /// Intertext — break out of alignment for a text paragraph
    Intertext(String),
    /// Label inside math (for align equation numbering)
    Label(String),
    /// Substack — vertically stacked lines (used under \sum)
    Substack(Vec<Vec<MathNode>>),
    /// Styled text — text with explicit font (for \textbf, \textit, etc. in math)
    StyledText(String, crate::font::FontId),
    /// \vphantom — zero width, keeps height/depth
    VPhantom(Vec<MathNode>),
    /// \hphantom — zero height/depth, keeps width
    HPhantom(Vec<MathNode>),
    /// \pmod{X} → renders as "(mod X)" with thin space before
    Pmod(Vec<MathNode>),
    /// \bmod → renders as "mod" binary operator
    Bmod,
    /// \pod{X} → renders as "(X)" with thin space before
    Pod(Vec<MathNode>),
    /// \mathrel{X} — relation spacing wrapper
    MathRel(Vec<MathNode>),
    /// \mathbin{X} — binary operator spacing wrapper
    MathBin(Vec<MathNode>),
    /// \rule{width}{height} in math mode — inline filled rectangle
    Rule { width: f32, height: f32 },
    /// \middle delimiter
    Middle(String),
}

#[derive(Debug, Clone, Copy)]
pub enum MatrixStyle {
    Plain,
    Parenthesized,
    Bracketed,
    Braced,
    VerticalBar,
    DoubleBar,
}

#[derive(Debug, Clone, Copy)]
pub enum MathFontType {
    Blackboard,
    Calligraphic,
    Fraktur,
    Script,
    SansSerif,
    BoldMath,
}

#[derive(Debug, Clone, Copy)]
pub enum MathStyleType {
    Display,
    Text,
    Script,
    ScriptScript,
}

#[derive(Debug, Clone)]
pub enum AccentType {
    Hat,
    Tilde,
    Bar,
    Vec,
    Dot,
    DDot,
    Breve,
    Check,
}

#[derive(Debug, Clone)]
pub enum OverType {
    Line,
    Brace,
    Arrow,
}

#[derive(Debug, Clone)]
pub enum UnderType {
    Line,
    Brace,
}
