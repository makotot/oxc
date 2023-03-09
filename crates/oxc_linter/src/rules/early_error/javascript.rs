#[allow(clippy::wildcard_imports)]
use oxc_ast::{ast::*, AstKind, Atom, Span};
use oxc_diagnostics::{
    miette::{self, Diagnostic},
    thiserror::Error,
};

use crate::{context::LintContext, rule::Rule, AstNode};

#[derive(Debug, Default, Clone)]
pub struct EarlyErrorJavaScript;

impl Rule for EarlyErrorJavaScript {
    #[allow(clippy::single_match)]
    fn run<'a>(&self, node: &AstNode<'a>, ctx: &LintContext<'a>) {
        match node.get().kind() {
            AstKind::PrivateIdentifier(ident) => check_private_identifier(ident, node, ctx),
            AstKind::NumberLiteral(lit) => check_number_literal(lit, node, ctx),
            AstKind::StringLiteral(lit) => check_string_literal(lit, node, ctx),
            AstKind::RegExpLiteral(lit) => check_regexp_literal(lit, ctx),
            AstKind::BreakStatement(lit) => check_break_statement(lit, node, ctx),
            _ => {}
        }
    }
}

fn check_private_identifier<'a>(
    ident: &PrivateIdentifier,
    node: &AstNode<'a>,
    ctx: &LintContext<'a>,
) {
    // Ignore private identifier declaration inside class
    if matches!(ctx.parent_kind(node), AstKind::PropertyKey(_)) {
        return;
    }

    // Find enclosing classes
    let mut classes = vec![];
    for node_id in ctx.ancestors(node).skip(1) {
        let kind = ctx.kind(node_id);
        if let AstKind::Class(class) = kind {
            classes.push(class);
        }
        // stop lookup when the class is a heritage, e.g.
        // `class C extends class extends class { x = this.#foo; } {} { #foo }`
        // `class C extends function() { x = this.#foo; } { #foo }`
        if matches!(kind, AstKind::ClassHeritage(_)) {
            break;
        }
    }

    if classes.is_empty() {
        #[derive(Debug, Error, Diagnostic)]
        #[error("Private identifier '#{0:?}' is not allowed outside class bodies")]
        #[diagnostic()]
        struct PrivateNotInClass(Atom, #[label] Span);
        ctx.diagnostic(PrivateNotInClass(ident.name.clone(), ident.span));
        return;
    };

    // Check private identifier declarations in class.
    // This implementations does a simple lookup for private identifier declarations inside a class.
    // Performance can be improved by storing private identifiers for each class inside a lookup table,
    // but there are not many private identifiers in the wild so we should be good fow now.
    let found_private_ident = classes.iter().any(|class| {
        class.body.body.iter().any(|def| {
            // let key = match def {
            // ClassElement::PropertyDefinition(def) => &def.key,
            // ClassElement::MethodDefinition(def) => &def.key,
            // _ => return false,
            // };
            if let Some(key) = def.property_key()
                && let PropertyKey::PrivateIdentifier(prop_ident) = key {
                return prop_ident.name == ident.name;
            }
            false
        })
    });

    if !found_private_ident {
        #[derive(Debug, Error, Diagnostic)]
        #[error("Private field '{0:?}' must be declared in an enclosing class")]
        #[diagnostic()]
        struct PrivateFieldUndeclared(Atom, #[label] Span);
        ctx.diagnostic(PrivateFieldUndeclared(ident.name.clone(), ident.span));
    }
}

#[derive(Debug, Error, Diagnostic)]
#[error("'0'-prefixed octal literals and octal escape sequences are deprecated")]
#[diagnostic(help("for octal literals use the '0o' prefix instead"))]
struct LegacyOctal(#[label] Span);

fn check_number_literal(lit: &NumberLiteral, node: &AstNode, ctx: &LintContext) {
    // NumericLiteral :: LegacyOctalIntegerLiteral
    // DecimalIntegerLiteral :: NonOctalDecimalIntegerLiteral
    // * It is a Syntax Error if the source text matched by this production is strict mode code.
    fn leading_zero(s: &str) -> bool {
        let mut chars = s.bytes();
        if let Some(first) = chars.next() {
            if let Some(second) = chars.next() {
                return first == b'0' && second.is_ascii_digit();
            }
        }
        false
    }

    if ctx.strict_mode(node) {
        match lit.base {
            NumberBase::Octal if leading_zero(lit.raw) => {
                ctx.diagnostic(LegacyOctal(lit.span));
            }
            NumberBase::Decimal if leading_zero(lit.raw) => {
                #[derive(Debug, Error, Diagnostic)]
                #[error("Decimals with leading zeros are not allowed in strict mode")]
                #[diagnostic(help("remove the leading zero"))]
                struct LeadingZeroDecimal(#[label] Span);

                ctx.diagnostic(LeadingZeroDecimal(lit.span));
            }
            _ => {}
        }
    }
}

fn check_string_literal<'a>(lit: &StringLiteral, node: &AstNode<'a>, ctx: &LintContext<'a>) {
    // 12.9.4.1 Static Semantics: Early Errors
    // EscapeSequence ::
    //   LegacyOctalEscapeSequence
    //   NonOctalDecimalEscapeSequence
    // It is a Syntax Error if the source text matched by this production is strict mode code.
    let raw = lit.span.source_text(ctx.source_text());
    if ctx.strict_mode(node) && raw.len() != lit.value.len() {
        let mut chars = raw.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '\\' {
                match chars.next() {
                    Some('0') => {
                        if chars.peek().is_some_and(|c| ('1'..='9').contains(c)) {
                            ctx.diagnostic(LegacyOctal(lit.span));
                            return;
                        }
                    }
                    Some('1'..='7') => {
                        ctx.diagnostic(LegacyOctal(lit.span));
                        return;
                    }
                    Some('8'..='9') => {
                        #[derive(Debug, Error, Diagnostic)]
                        #[error("Invalid escape sequence")]
                        #[diagnostic(help("\\8 and \\9 are not allowed in strict mode"))]
                        struct NonOctalDecimalEscapeSequence(#[label] Span);
                        ctx.diagnostic(NonOctalDecimalEscapeSequence(lit.span));
                        return;
                    }
                    _ => {}
                }
            }
        }
    }
}

fn check_regexp_literal(lit: &RegExpLiteral, ctx: &LintContext) {
    #[derive(Debug, Error, Diagnostic)]
    #[error("The 'u' and 'v' regular expression flags cannot be enabled at the same time")]
    #[diagnostic()]
    struct RegExpFlagUAndV(#[label] Span);

    let flags = lit.regex.flags;
    if flags.contains(RegExpFlags::U | RegExpFlags::V) {
        ctx.diagnostic(RegExpFlagUAndV(lit.span));
    }
}

fn check_break_statement<'a>(stmt: &BreakStatement, node: &AstNode<'a>, ctx: &LintContext<'a>) {
    #[derive(Debug, Error, Diagnostic)]
    #[error("Illegal break statement")]
    #[diagnostic(help(
        "A `break` statement can only be used within an enclosing iteration or switch statement."
    ))]
    struct InvalidBreak(#[label] Span);

    #[derive(Debug, Error, Diagnostic)]
    #[error("Illegal continue statement: no surrounding iteration statement")]
    #[diagnostic(help(
        "A `continue` statement can only be used within an enclosing `for`, `while` or `do while` "
    ))]
    struct InvalidContinue(#[label] Span);

    #[derive(Debug, Error, Diagnostic)]
    #[error("Use of undefined label")]
    #[diagnostic()]
    struct InvalidLabelTarget(#[label("This label is used, but not defined")] Span);

    #[derive(Debug, Error, Diagnostic)]
    #[error("Jump target cannot cross function boundary.")]
    #[diagnostic()]
    struct InvalidLabelJumpTarget(#[label] Span);

    // It is a Syntax Error if this BreakStatement is not nested, directly or indirectly (but not crossing function or static initialization block boundaries), within an IterationStatement or a SwitchStatement.
    for node_id in ctx.ancestors(node).skip(1) {
        match ctx.kind(node_id) {
            AstKind::Program(_) => {
                stmt.label.as_ref().map_or_else(
                    || ctx.diagnostic(InvalidBreak(stmt.span)),
                    |label| ctx.diagnostic(InvalidLabelTarget(label.span)),
                );
                return;
            }
            AstKind::Function(_) | AstKind::StaticBlock(_) => {
                stmt.label.as_ref().map_or_else(
                    || ctx.diagnostic(InvalidContinue(stmt.span)),
                    |label| ctx.diagnostic(InvalidLabelJumpTarget(label.span)),
                );
                return;
            }
            AstKind::LabeledStatement(labeled_statement) => {
                if let Some(label) = &stmt.label
                    && label.name == labeled_statement.label.name {
                    break;
                }
            }
            kind if (kind.is_iteration_statement()
                || matches!(kind, AstKind::SwitchStatement(_)))
                && stmt.label.is_none() =>
            {
                break;
            }
            _ => {}
        }
    }
}
