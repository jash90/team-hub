//! TQL, the issue query language:
//! `project = WEB AND status in ("In Progress", Done) AND assignee = currentUser() ORDER BY priority DESC`.
//! This module only parses; the application turns the tree into SQL.

use serde::Serialize;
use ts_rs::TS;

#[derive(Debug, Clone, PartialEq)]
pub enum QueryValue {
    Text(String),
    Number(f64),
    Func(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClauseOp {
    Eq,
    NotEq,
    Like,
    NotLike,
    Less,
    LessEq,
    Greater,
    GreaterEq,
    In,
    NotIn,
    IsEmpty,
    IsNotEmpty,
}

impl ClauseOp {
    pub fn is_negative(self) -> bool {
        matches!(self, Self::NotEq | Self::NotLike | Self::NotIn | Self::IsNotEmpty)
    }

    pub fn is_comparison(self) -> bool {
        matches!(self, Self::Less | Self::LessEq | Self::Greater | Self::GreaterEq)
    }

    pub fn sql(self) -> &'static str {
        match self {
            Self::Eq | Self::In | Self::Like => "=",
            Self::NotEq | Self::NotIn | Self::NotLike => "!=",
            Self::Less => "<",
            Self::LessEq => "<=",
            Self::Greater => ">",
            Self::GreaterEq => ">=",
            Self::IsEmpty => "IS NULL",
            Self::IsNotEmpty => "IS NOT NULL",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Clause {
    pub field: String,
    pub op: ClauseOp,
    pub values: Vec<QueryValue>,
    pub position: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub enum QueryNode {
    And(Vec<QueryNode>),
    Or(Vec<QueryNode>),
    Not(Box<QueryNode>),
    Clause(Clause),
}

#[derive(Debug, Clone, PartialEq)]
pub struct OrderBy {
    pub field: String,
    pub descending: bool,
    pub position: usize,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ParsedQuery {
    pub filter: Option<QueryNode>,
    pub order_by: Vec<OrderBy>,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[error("{message}")]
pub struct QueryError {
    pub message: String,
    pub position: usize,
}

impl QueryError {
    pub fn new(message: impl Into<String>, position: usize) -> Self {
        Self { message: message.into(), position }
    }
}

#[derive(Debug, Clone, PartialEq)]
enum TokenKind {
    Word(String),
    Quoted(String),
    Op(ClauseOp),
    Open,
    Close,
    Comma,
    End,
}

#[derive(Debug, Clone, PartialEq)]
struct Token {
    kind: TokenKind,
    position: usize,
}

const OPERATORS: [(&str, ClauseOp); 8] = [
    ("!=", ClauseOp::NotEq),
    ("!~", ClauseOp::NotLike),
    ("<=", ClauseOp::LessEq),
    (">=", ClauseOp::GreaterEq),
    ("=", ClauseOp::Eq),
    ("~", ClauseOp::Like),
    ("<", ClauseOp::Less),
    (">", ClauseOp::Greater),
];

fn ends_word(character: char) -> bool {
    character.is_whitespace() || "(),=!~<>\"'".contains(character)
}

fn tokenize(input: &str) -> Result<Vec<Token>, QueryError> {
    let characters: Vec<(usize, char)> = input.char_indices().collect();
    let mut tokens = Vec::new();
    let mut index = 0;

    while index < characters.len() {
        let (position, character) = characters[index];

        if character.is_whitespace() {
            index += 1;
            continue;
        }

        let simple = match character {
            '(' => Some(TokenKind::Open),
            ')' => Some(TokenKind::Close),
            ',' => Some(TokenKind::Comma),
            _ => None,
        };

        if let Some(kind) = simple {
            tokens.push(Token { kind, position });
            index += 1;
            continue;
        }

        if character == '"' || character == '\'' {
            let mut text = String::new();
            let mut cursor = index + 1;

            loop {
                let Some(&(_, next)) = characters.get(cursor) else {
                    return Err(QueryError::new("Unterminated quoted text", position));
                };

                if next == character {
                    break;
                }

                if next == '\\' && cursor + 1 < characters.len() {
                    cursor += 1;
                }

                text.push(characters[cursor].1);
                cursor += 1;
            }

            tokens.push(Token { kind: TokenKind::Quoted(text), position });
            index = cursor + 1;
            continue;
        }

        if let Some((text, op)) = OPERATORS.iter().find(|(text, _)| input[position..].starts_with(text)) {
            tokens.push(Token { kind: TokenKind::Op(*op), position });
            index += text.len();
            continue;
        }

        if character == '!' {
            return Err(QueryError::new("Unexpected \"!\"", position));
        }

        let mut cursor = index;

        while cursor < characters.len() && !ends_word(characters[cursor].1) {
            cursor += 1;
        }

        let end = characters.get(cursor).map_or(input.len(), |(offset, _)| *offset);
        tokens.push(Token { kind: TokenKind::Word(input[position..end].to_string()), position });
        index = cursor;
    }

    tokens.push(Token { kind: TokenKind::End, position: input.len() });

    Ok(tokens)
}

struct Parser {
    tokens: Vec<Token>,
    cursor: usize,
}

impl Parser {
    fn peek(&self, offset: usize) -> &Token {
        &self.tokens[(self.cursor + offset).min(self.tokens.len() - 1)]
    }

    fn next(&mut self) -> Token {
        let token = self.peek(0).clone();
        self.cursor += 1;

        token
    }

    fn is_keyword(&self, word: &str, offset: usize) -> bool {
        matches!(&self.peek(offset).kind, TokenKind::Word(text) if text.eq_ignore_ascii_case(word))
    }

    fn is_order_by(&self) -> bool {
        self.is_keyword("order", 0) && self.is_keyword("by", 1)
    }

    fn eat(&mut self, kind: &TokenKind) -> bool {
        if &self.peek(0).kind == kind {
            self.cursor += 1;
            return true;
        }

        false
    }

    fn expect_close(&mut self) -> Result<(), QueryError> {
        let token = self.next();

        if token.kind != TokenKind::Close {
            return Err(QueryError::new("Expected \")\"", token.position));
        }

        Ok(())
    }

    fn parse(mut self) -> Result<ParsedQuery, QueryError> {
        let mut filter = None;

        if self.peek(0).kind != TokenKind::End && !self.is_order_by() {
            filter = Some(self.parse_or()?);
        }

        let order_by = if self.is_order_by() { self.parse_order_by()? } else { Vec::new() };
        let rest = self.peek(0);

        if rest.kind != TokenKind::End {
            return Err(QueryError::new("Expected AND, OR or ORDER BY", rest.position));
        }

        Ok(ParsedQuery { filter, order_by })
    }

    fn parse_order_by(&mut self) -> Result<Vec<OrderBy>, QueryError> {
        self.cursor += 2;
        let mut order_by = Vec::new();

        loop {
            let token = self.next();
            let TokenKind::Word(field) = token.kind else {
                return Err(QueryError::new("Expected a field to order by", token.position));
            };
            let mut descending = false;

            if self.is_keyword("asc", 0) || self.is_keyword("desc", 0) {
                descending = self.is_keyword("desc", 0);
                self.cursor += 1;
            }

            order_by.push(OrderBy { field, descending, position: token.position });

            if !self.eat(&TokenKind::Comma) {
                break;
            }
        }

        Ok(order_by)
    }

    fn parse_or(&mut self) -> Result<QueryNode, QueryError> {
        let mut items = vec![self.parse_and()?];

        while self.is_keyword("or", 0) {
            self.cursor += 1;
            items.push(self.parse_and()?);
        }

        Ok(if items.len() == 1 { items.remove(0) } else { QueryNode::Or(items) })
    }

    fn parse_and(&mut self) -> Result<QueryNode, QueryError> {
        let mut items = vec![self.parse_not()?];

        while self.is_keyword("and", 0) {
            self.cursor += 1;
            items.push(self.parse_not()?);
        }

        Ok(if items.len() == 1 { items.remove(0) } else { QueryNode::And(items) })
    }

    fn parse_not(&mut self) -> Result<QueryNode, QueryError> {
        if self.is_keyword("not", 0) {
            self.cursor += 1;

            return Ok(QueryNode::Not(Box::new(self.parse_not()?)));
        }

        if self.eat(&TokenKind::Open) {
            let inner = self.parse_or()?;
            self.expect_close()?;

            return Ok(inner);
        }

        self.parse_clause()
    }

    fn parse_clause(&mut self) -> Result<QueryNode, QueryError> {
        let field_token = self.next();
        let TokenKind::Word(field) = field_token.kind else {
            return Err(QueryError::new("Expected a field name", field_token.position));
        };
        let position = field_token.position;
        let operator = self.next();
        let clause = |op, values| Ok(QueryNode::Clause(Clause { field: field.clone(), op, values, position }));

        match &operator.kind {
            TokenKind::Op(op) => {
                let value = self.parse_value()?;

                clause(*op, vec![value])
            }
            TokenKind::Word(word) if word.eq_ignore_ascii_case("in") => {
                let values = self.parse_list()?;

                clause(ClauseOp::In, values)
            }
            TokenKind::Word(word) if word.eq_ignore_ascii_case("not") && self.is_keyword("in", 0) => {
                self.cursor += 1;
                let values = self.parse_list()?;

                clause(ClauseOp::NotIn, values)
            }
            TokenKind::Word(word) if word.eq_ignore_ascii_case("is") => {
                let negated = self.is_keyword("not", 0);

                if negated {
                    self.cursor += 1;
                }

                if !self.is_keyword("empty", 0) && !self.is_keyword("null", 0) {
                    return Err(QueryError::new("Expected EMPTY after IS", self.peek(0).position));
                }

                self.cursor += 1;

                clause(if negated { ClauseOp::IsNotEmpty } else { ClauseOp::IsEmpty }, Vec::new())
            }
            _ => Err(QueryError::new(format!("Expected an operator after \"{field}\""), operator.position)),
        }
    }

    fn parse_list(&mut self) -> Result<Vec<QueryValue>, QueryError> {
        if !self.eat(&TokenKind::Open) {
            // `sprint in openSprints()`: a function standing in for the whole list.
            return Ok(vec![self.parse_value()?]);
        }

        let mut values = Vec::new();

        loop {
            values.push(self.parse_value()?);

            if !self.eat(&TokenKind::Comma) {
                break;
            }
        }

        self.expect_close()?;

        Ok(values)
    }

    fn parse_value(&mut self) -> Result<QueryValue, QueryError> {
        let token = self.next();

        match token.kind {
            TokenKind::Quoted(text) => Ok(QueryValue::Text(text)),
            TokenKind::Word(word) => {
                if self.peek(0).kind == TokenKind::Open && self.peek(1).kind == TokenKind::Close {
                    self.cursor += 2;

                    return Ok(QueryValue::Func(word.to_lowercase()));
                }

                match word.parse::<f64>() {
                    Ok(number) if word.chars().all(|character| character.is_ascii_digit() || character == '.') => Ok(QueryValue::Number(number)),
                    _ => Ok(QueryValue::Text(word)),
                }
            }
            _ => Err(QueryError::new("Expected a value", token.position)),
        }
    }
}

pub fn parse_query(input: &str) -> Result<ParsedQuery, QueryError> {
    Parser { tokens: tokenize(input)?, cursor: 0 }.parse()
}

/// True when the text has no query syntax at all, so it is a plain text search.
pub fn looks_like_plain_text(input: &str) -> bool {
    let lower = format!(" {} ", input.to_lowercase());

    !input.trim().is_empty()
        && !input.contains(['=', '~', '<', '>', '!', '(', ')'])
        && !lower.contains(" is ")
        && !lower.contains(" in ")
        && !lower.contains(" order by ")
}

/// A value as it can be written back into a query: bare when safe, quoted otherwise.
pub fn quote_value(text: &str) -> String {
    const RESERVED: [&str; 9] = ["and", "or", "not", "in", "is", "empty", "null", "order", "by"];
    let bare = !text.is_empty()
        && text.chars().all(|character| character.is_alphanumeric() || "_.@+-".contains(character))
        && !RESERVED.contains(&text.to_lowercase().as_str());

    if bare {
        text.to_string()
    } else {
        format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
    }
}

/// What the query editor needs to explain and complete a field.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct QueryFieldInfo {
    pub name: String,
    pub aliases: Vec<String>,
    pub kind: String,
    pub example: String,
}

impl QueryFieldInfo {
    pub fn new(name: &str, aliases: &[&str], kind: &str, example: &str) -> Self {
        Self {
            name: name.to_string(),
            aliases: aliases.iter().map(|alias| alias.to_string()).collect(),
            kind: kind.to_string(),
            example: example.to_string(),
        }
    }
}

pub const QUERY_FUNCTIONS: [&str; 6] = ["currentUser()", "openSprints()", "now()", "startOfDay()", "startOfWeek()", "startOfMonth()"];

#[cfg(test)]
mod tests {
    use super::*;

    fn clause(node: &QueryNode) -> &Clause {
        match node {
            QueryNode::Clause(clause) => clause,
            other => panic!("expected a clause, got {other:?}"),
        }
    }

    #[test]
    fn parses_clauses_lists_and_order() {
        let query =
            parse_query("project = WEB AND status in (\"In Progress\", Done) AND assignee = currentUser() ORDER BY priority DESC, created").unwrap();
        let QueryNode::And(items) = query.filter.unwrap() else { panic!("expected AND") };

        assert_eq!(items.len(), 3);
        assert_eq!(clause(&items[1]).op, ClauseOp::In);
        assert_eq!(clause(&items[1]).values, vec![QueryValue::Text("In Progress".into()), QueryValue::Text("Done".into())]);
        assert_eq!(clause(&items[2]).values, vec![QueryValue::Func("currentuser".into())]);
        assert_eq!(query.order_by.len(), 2);
        assert!(query.order_by[0].descending && !query.order_by[1].descending);
    }

    #[test]
    fn or_binds_looser_than_and_and_parentheses_override() {
        let query = parse_query("a = 1 OR b = 2 AND c = 3").unwrap();
        let QueryNode::Or(items) = query.filter.unwrap() else { panic!("expected OR") };

        assert!(matches!(items[1], QueryNode::And(_)));

        let query = parse_query("(a = 1 OR b = 2) AND NOT c = 3").unwrap();
        let QueryNode::And(items) = query.filter.unwrap() else { panic!("expected AND") };

        assert!(matches!(items[0], QueryNode::Or(_)) && matches!(items[1], QueryNode::Not(_)));
    }

    #[test]
    fn parses_emptiness_negation_and_relative_dates() {
        let query = parse_query("assignee is empty and sprint not in openSprints() and created >= -7d and points > 3").unwrap();
        let QueryNode::And(items) = query.filter.unwrap() else { panic!("expected AND") };

        assert_eq!(clause(&items[0]).op, ClauseOp::IsEmpty);
        assert_eq!(clause(&items[1]).op, ClauseOp::NotIn);
        assert_eq!(clause(&items[2]).values, vec![QueryValue::Text("-7d".into())]);
        assert_eq!(clause(&items[3]).values, vec![QueryValue::Number(3.0)]);
    }

    #[test]
    fn order_by_alone_is_a_query() {
        let query = parse_query("order by updated desc").unwrap();

        assert!(query.filter.is_none());
        assert_eq!(query.order_by[0].field, "updated");
    }

    #[test]
    fn reports_where_it_went_wrong() {
        assert_eq!(parse_query("status = ").unwrap_err().position, 9);
        assert_eq!(parse_query("status \"x\"").unwrap_err().position, 7);
        assert!(parse_query("status = \"open").is_err());
        assert!(parse_query("status = Done priority = High").is_err());
    }

    #[test]
    fn tells_plain_text_from_queries() {
        assert!(looks_like_plain_text("login error on safari"));
        assert!(!looks_like_plain_text("status = Done"));
        assert!(!looks_like_plain_text("assignee is empty"));
        assert!(!looks_like_plain_text("  "));
    }

    #[test]
    fn quotes_only_when_needed() {
        assert_eq!(quote_value("Done"), "Done");
        assert_eq!(quote_value("In Progress"), "\"In Progress\"");
        assert_eq!(quote_value("order"), "\"order\"");
        assert_eq!(quote_value("say \"hi\""), "\"say \\\"hi\\\"\"");
    }
}
