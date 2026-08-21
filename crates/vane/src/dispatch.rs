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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BareAction {
    InitHint,
    Doctor,
    Status,
}

/// Bare `vane` on a TTY (spec §4.1). Non-TTY never reaches here (main prints help).
pub fn decide_bare(initialized: bool, daemon_running: bool) -> BareAction {
    if !initialized {
        BareAction::InitHint
    } else if !daemon_running {
        BareAction::Doctor
    } else {
        BareAction::Status
    }
}
