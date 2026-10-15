//! A bounded, exhaustive model of one escrow session.
//!
//! This model checks protocol-level assumptions. It does not prove that the
//! Soroban implementation refines the model; the traceability table in
//! `docs/escrow-invariants.md` identifies the implementation tests that cover
//! each modeled action.

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, VecDeque};

    const MAX_DEPTH: usize = 7;
    const EXPIRY_LEDGER: u8 = 2;

    #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
    enum Status {
        Absent,
        Open,
        Settled,
        Cancelled,
        Expired,
    }

    #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
    struct State {
        initialized: bool,
        asset_supported: bool,
        status: Status,
        ledger: u8,
        cap: u8,
        contract_balance: u8,
        liability: u8,
        seller_paid: u8,
        buyer_refunded: u8,
        other_refunded: u8,
        deposited: u8,
        terminal_transitions: u8,
    }

    impl Default for State {
        fn default() -> Self {
            Self {
                initialized: false,
                asset_supported: false,
                status: Status::Absent,
                ledger: 0,
                cap: 0,
                contract_balance: 0,
                liability: 0,
                seller_paid: 0,
                buyer_refunded: 0,
                other_refunded: 0,
                deposited: 0,
                terminal_transitions: 0,
            }
        }
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum Action {
        Initialize { admin_authorized: bool },
        Create { buyer_authorized: bool, cap: u8 },
        Settle { seller_authorized: bool, amount: u8 },
        Cancel { buyer_authorized: bool },
        AdvanceToExpiry,
        Recover,
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum Rejection {
        Unauthorized,
        InvalidState,
        InvalidAmount,
        Expired,
        NotExpired,
        UnsupportedAsset,
    }

    const ACTIONS: [Action; 17] = [
        Action::Initialize {
            admin_authorized: false,
        },
        Action::Initialize {
            admin_authorized: true,
        },
        Action::Create {
            buyer_authorized: false,
            cap: 1,
        },
        Action::Create {
            buyer_authorized: true,
            cap: 0,
        },
        Action::Create {
            buyer_authorized: true,
            cap: 1,
        },
        Action::Create {
            buyer_authorized: true,
            cap: 2,
        },
        Action::Settle {
            seller_authorized: false,
            amount: 1,
        },
        Action::Settle {
            seller_authorized: true,
            amount: 0,
        },
        Action::Settle {
            seller_authorized: true,
            amount: 1,
        },
        Action::Settle {
            seller_authorized: true,
            amount: 2,
        },
        Action::Settle {
            seller_authorized: true,
            amount: 3,
        },
        Action::Cancel {
            buyer_authorized: false,
        },
        Action::Cancel {
            buyer_authorized: true,
        },
        Action::AdvanceToExpiry,
        Action::Recover,
        Action::Create {
            buyer_authorized: false,
            cap: 2,
        },
        Action::Settle {
            seller_authorized: false,
            amount: 2,
        },
    ];

    fn apply(state: &mut State, action: Action) -> Result<(), Rejection> {
        match action {
            Action::Initialize { admin_authorized } => {
                if !admin_authorized {
                    return Err(Rejection::Unauthorized);
                }
                if state.initialized {
                    return Err(Rejection::InvalidState);
                }
                state.initialized = true;
                state.asset_supported = true;
            }
            Action::Create {
                buyer_authorized,
                cap,
            } => {
                if !state.initialized || !state.asset_supported {
                    return Err(Rejection::UnsupportedAsset);
                }
                if !buyer_authorized {
                    return Err(Rejection::Unauthorized);
                }
                if state.status != Status::Absent {
                    return Err(Rejection::InvalidState);
                }
                if cap == 0 {
                    return Err(Rejection::InvalidAmount);
                }
                state.status = Status::Open;
                state.cap = cap;
                state.contract_balance = cap;
                state.liability = cap;
                state.deposited = cap;
            }
            Action::Settle {
                seller_authorized,
                amount,
            } => {
                require_open(state)?;
                if state.ledger >= EXPIRY_LEDGER {
                    return Err(Rejection::Expired);
                }
                if !seller_authorized {
                    return Err(Rejection::Unauthorized);
                }
                if amount == 0 || amount > state.cap {
                    return Err(Rejection::InvalidAmount);
                }
                state.status = Status::Settled;
                state.seller_paid = amount;
                state.buyer_refunded = state.cap - amount;
                finish_terminal_transition(state);
            }
            Action::Cancel { buyer_authorized } => {
                require_open(state)?;
                if state.ledger >= EXPIRY_LEDGER {
                    return Err(Rejection::Expired);
                }
                if !buyer_authorized {
                    return Err(Rejection::Unauthorized);
                }
                state.status = Status::Cancelled;
                state.buyer_refunded = state.cap;
                finish_terminal_transition(state);
            }
            Action::AdvanceToExpiry => {
                state.ledger = EXPIRY_LEDGER;
            }
            Action::Recover => {
                require_open(state)?;
                if state.ledger < EXPIRY_LEDGER {
                    return Err(Rejection::NotExpired);
                }
                state.status = Status::Expired;
                state.buyer_refunded = state.cap;
                finish_terminal_transition(state);
            }
        }
        Ok(())
    }

    fn require_open(state: &State) -> Result<(), Rejection> {
        if state.status == Status::Open {
            Ok(())
        } else {
            Err(Rejection::InvalidState)
        }
    }

    fn finish_terminal_transition(state: &mut State) {
        state.contract_balance = 0;
        state.liability = 0;
        state.terminal_transitions += 1;
    }

    fn invariant_violation(state: &State) -> Option<&'static str> {
        if state.terminal_transitions > 1 {
            return Some("more than one terminal transition succeeded");
        }
        if state.seller_paid > state.cap {
            return Some("seller payout exceeded the session cap");
        }
        if state.other_refunded != 0 {
            return Some("refund was assigned to an address other than the buyer");
        }
        if state.contract_balance + state.seller_paid + state.buyer_refunded != state.deposited {
            return Some("deposited value was not conserved");
        }
        match state.status {
            Status::Absent => {
                if state.deposited != 0 || state.liability != 0 {
                    return Some("absent session retained value or liability");
                }
            }
            Status::Open => {
                if state.liability != state.cap || state.contract_balance != state.cap {
                    return Some("open-session liability did not equal escrowed cap");
                }
            }
            Status::Settled | Status::Cancelled | Status::Expired => {
                if state.liability != 0
                    || state.contract_balance != 0
                    || state.terminal_transitions != 1
                {
                    return Some("terminal session retained escrow or invalid transition count");
                }
            }
        }
        None
    }

    fn trace_text(trace: &[Action]) -> String {
        trace
            .iter()
            .enumerate()
            .map(|(index, action)| format!("{}. {action:?}", index + 1))
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn bounded_state_space_preserves_escrow_invariants() {
        let initial = State::default();
        let mut queue = VecDeque::from([(initial, Vec::new())]);
        let mut visited = HashMap::from([(initial, Vec::new())]);

        while let Some((state, trace)) = queue.pop_front() {
            if trace.len() == MAX_DEPTH {
                continue;
            }
            for action in ACTIONS {
                let mut next = state;
                let mut next_trace = trace.clone();
                next_trace.push(action);
                let result = apply(&mut next, action);
                if result.is_err() && next != state {
                    panic!(
                        "rejected action mutated state\n{}\nrejection: {result:?}",
                        trace_text(&next_trace)
                    );
                }
                if let Some(violation) = invariant_violation(&next) {
                    panic!(
                        "model invariant failed: {violation}\nminimal counterexample:\n{}\nstate: {next:?}",
                        trace_text(&next_trace)
                    );
                }
                if let std::collections::hash_map::Entry::Vacant(entry) = visited.entry(next) {
                    entry.insert(next_trace.clone());
                    queue.push_back((next, next_trace));
                }
            }
        }

        assert!(
            visited.len() >= 20,
            "model exploration was unexpectedly small"
        );
    }

    #[test]
    fn counterexample_trace_format_is_deterministic() {
        let trace = [
            Action::Initialize {
                admin_authorized: true,
            },
            Action::Create {
                buyer_authorized: true,
                cap: 2,
            },
            Action::Settle {
                seller_authorized: true,
                amount: 3,
            },
        ];

        assert_eq!(
            trace_text(&trace),
            "1. Initialize { admin_authorized: true }\n\
             2. Create { buyer_authorized: true, cap: 2 }\n\
             3. Settle { seller_authorized: true, amount: 3 }"
        );
    }
}
