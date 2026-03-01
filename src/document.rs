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
    pub commands: Vec<(String, Vec<Node>)>,
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
            commands: Vec::new(),
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
        // US Letter: 8.5 x 11 inches = 612 x 792 points
        PageSetup {
            width: 612.0,
            height: 792.0,
            margin_top: 72.0,    // 1 inch
            margin_bottom: 72.0,
            margin_left: 72.0,
            margin_right: 72.0,
            header_height: 0.0,
            footer_height: 20.0,  // space for page number
            columns: 1,
            column_sep: 18.0,
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

    /// Horizontal space
    HSpace(f32),

    /// Vertical space
    VSpace(f32),

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
    DisplayMath(Vec<MathNode>),

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

    /// Footnote
    Footnote(Vec<Node>),

    /// Cross-reference
    Label(String),
    Ref(String),
    Citation(String),
    /// Bibliography item marker
    BibItem(String),

    /// Horizontal rule
    HRule,

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

    /// Raw content (pass-through)
    Raw(String),

    /// Group (braces)
    Group(Vec<Node>),

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
            SectionLevel::Subsubsection => base * 1.1,
            SectionLevel::Paragraph => base,
            SectionLevel::Subparagraph => base,
        }
    }

    pub fn spacing_before(&self) -> f32 {
        match self {
            SectionLevel::Part => 36.0,
            SectionLevel::Chapter => 30.0,
            SectionLevel::Section => 22.0,
            SectionLevel::Subsection => 16.0,
            SectionLevel::Subsubsection => 12.0,
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
}

#[derive(Debug, Clone)]
pub struct TableCell {
    pub content: Vec<Node>,
    pub colspan: u32,
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
    Sum { lower: Option<Vec<MathNode>>, upper: Option<Vec<MathNode>> },
    Integral { lower: Option<Vec<MathNode>>, upper: Option<Vec<MathNode>> },
    Product { lower: Option<Vec<MathNode>>, upper: Option<Vec<MathNode>> },
    Matrix { rows: Vec<Vec<Vec<MathNode>>>, style: MatrixStyle },
    Accent { base: Vec<MathNode>, accent_type: AccentType },
    Over { content: Vec<MathNode>, over_type: OverType },
    Under { content: Vec<MathNode>, under_type: UnderType },
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
