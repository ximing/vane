/// Pure dispatch decisions, unit-testable without a TTY or daemon (spec §7.1).

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueryArg {
    Run(String),
    Prompt,
    MissingError,
}

pub fn decide_query_arg(q: Option<String>, tty: bool) -> QueryArg {
    match (q, tty) {
        (Some(q), _) => QueryArg::Run(q),
        (None, true) => QueryArg::Prompt,
        (None, false) => QueryArg::MissingError,
    }
}
