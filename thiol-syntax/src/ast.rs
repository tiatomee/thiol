use crate::Loc;

pub type Identifier = String;

#[derive(Debug, Copy, Clone)]
pub enum Literal {
    Integer(i128),
    Float(f64),
}

#[derive(Debug, Clone)]
pub enum Expression {
    Literal(Literal),
    Variable(Identifier),
    Call {
        base: Box<Loc<Expression>>,
        args: Vec<(Option<Loc<Identifier>>, Loc<Expression>)>,
    },
    DotCall {
        base: Box<Loc<Expression>>,
        name: Loc<Identifier>,
        args: Vec<(Option<Loc<Identifier>>, Loc<Expression>)>,
    },
    InfixOp {
        op: InfixOp,
        args: Box<[Loc<Expression>; 2]>,
    },
    PrefixOp {
        op: PrefixOp,
        expr: Box<Loc<Expression>>,
    },
    Field {
        base: Box<Loc<Expression>>,
        name: Loc<Identifier>,
    },
    Index {
        base: Box<Loc<Expression>>,
        index: Box<Loc<Expression>>,
    },
}

#[derive(Debug, Clone)]
pub enum Statement {
    Becomes {
        // this needs to be checked to be a valid l-value
        lhs: Loc<Expression>,
        rhs: Loc<Expression>,
    },
}

#[derive(Debug, Copy, Clone)]
pub enum InfixOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,

    Gt,
    Gte,
    Lt,
    Lte,
    Eq,
    Neq,
}

#[derive(Debug, Copy, Clone)]
pub enum PrefixOp {
    Plus,
    Minus,
}