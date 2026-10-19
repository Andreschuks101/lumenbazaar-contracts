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
    enum AssetState {
        Absent,
        Active,
        Deprecated,
    }

    #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
    struct State {
        initialized: bool,
        admin: u8,
        pending_admin: Option<u8>,
        admin_rotation_expiry: u8,
        asset_state: AssetState,
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
                admin: 0,
                pending_admin: None,
                admin_rotation_expiry: 0,
                asset_state: AssetState::Absent,
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
        Initialize {
            admin_authorized: bool,
        },
        Create {
            buyer_authorized: bool,
            cap: u8,
        },
        Settle {
            seller_authorized: bool,
            amount: u8,
        },
        Cancel {
            buyer_authorized: bool,
        },
        AdvanceToExpiry,
        Recover,
        DeprecateAsset {
            caller: u8,
        },
        RemoveAsset {
            caller: u8,
        },
        ProposeAdmin {
            caller: u8,
            pending: u8,
            expires_at: u8,
        },
        AcceptAdmin {
            caller: u8,
        },
        CancelAdminRotation {
            caller: u8,
        },
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum Rejection {
        Unauthorized,
        InvalidState,
        InvalidAmount,
        Expired,
        NotExpired,
        UnsupportedAsset,
        AssetNotActive,
        AssetNotDeprecated,
        AssetLiabilityOutstanding,
        AdminRotationPending,
        AdminRotationNotFound,
        AdminRotationExpired,
        InvalidAdmin,
        InvalidAdminRotationExpiry,
    }

    const ACTIONS: [Action; 27] = [
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
        Action::DeprecateAsset { caller: 1 },
        Action::DeprecateAsset { caller: 3 },
        Action::RemoveAsset { caller: 1 },
        Action::RemoveAsset { caller: 3 },
        Action::ProposeAdmin {
            caller: 1,
            pending: 2,
            expires_at: 2,
        },
        Action::ProposeAdmin {
            caller: 3,
            pending: 2,
            expires_at: 2,
        },
        Action::AcceptAdmin { caller: 2 },
        Action::AcceptAdmin { caller: 3 },
        Action::CancelAdminRotation { caller: 1 },
        Action::CancelAdminRotation { caller: 3 },
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
                state.admin = 1;
                state.asset_state = AssetState::Active;
            }
            Action::Create {
                buyer_authorized,
                cap,
            } => {
                if !state.initialized || state.asset_state != AssetState::Active {
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
            Action::DeprecateAsset { caller } => {
                if !state.initialized {
                    return Err(Rejection::InvalidState);
                }
                if caller != state.admin {
                    return Err(Rejection::Unauthorized);
                }
                if state.asset_state != AssetState::Active {
                    return Err(Rejection::AssetNotActive);
                }
                state.asset_state = AssetState::Deprecated;
            }
            Action::RemoveAsset { caller } => {
                if !state.initialized {
                    return Err(Rejection::InvalidState);
                }
                if caller != state.admin {
                    return Err(Rejection::Unauthorized);
                }
                if state.asset_state != AssetState::Deprecated {
                    return Err(Rejection::AssetNotDeprecated);
                }
                if state.liability != 0 {
                    return Err(Rejection::AssetLiabilityOutstanding);
                }
                state.asset_state = AssetState::Absent;
            }
            Action::ProposeAdmin {
                caller,
                pending,
                expires_at,
            } => {
                if !state.initialized {
                    return Err(Rejection::InvalidState);
                }
                if caller != state.admin {
                    return Err(Rejection::Unauthorized);
                }
                if state.pending_admin.is_some() {
                    return Err(Rejection::AdminRotationPending);
                }
                if pending == 0 || pending == state.admin {
                    return Err(Rejection::InvalidAdmin);
                }
                if expires_at <= state.ledger {
                    return Err(Rejection::InvalidAdminRotationExpiry);
                }
                state.pending_admin = Some(pending);
                state.admin_rotation_expiry = expires_at;
            }
            Action::AcceptAdmin { caller } => {
                if !state.initialized {
                    return Err(Rejection::InvalidState);
                }
                let pending = state
                    .pending_admin
                    .ok_or(Rejection::AdminRotationNotFound)?;
                if state.ledger >= state.admin_rotation_expiry {
                    return Err(Rejection::AdminRotationExpired);
                }
                if caller != pending {
                    return Err(Rejection::Unauthorized);
                }
                state.admin = pending;
                state.pending_admin = None;
                state.admin_rotation_expiry = 0;
            }
            Action::CancelAdminRotation { caller } => {
                if !state.initialized {
                    return Err(Rejection::InvalidState);
                }
                if caller != state.admin {
                    return Err(Rejection::Unauthorized);
                }
                if state.pending_admin.is_none() {
                    return Err(Rejection::AdminRotationNotFound);
                }
                state.pending_admin = None;
                state.admin_rotation_expiry = 0;
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
        if state.asset_state == AssetState::Absent && state.initialized && state.liability != 0 {
            return Some("removed asset retained liability");
        }
        if !state.initialized && (state.admin != 0 || state.asset_state != AssetState::Absent) {
            return Some("uninitialized state had active admin or asset");
        }
        if state.initialized && state.admin == 0 {
            return Some("initialized contract had no admin");
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

    #[test]
    fn counterexample_deprecated_asset_rejects_new_sessions_preserving_terminal_exits() {
        let mut base_state = State::default();
        apply(
            &mut base_state,
            Action::Initialize {
                admin_authorized: true,
            },
        )
        .unwrap();
        apply(
            &mut base_state,
            Action::Create {
                buyer_authorized: true,
                cap: 2,
            },
        )
        .unwrap();
        apply(&mut base_state, Action::DeprecateAsset { caller: 1 }).unwrap();
        assert_eq!(base_state.asset_state, AssetState::Deprecated);

        // Deprecated asset rejects new session creation
        let mut new_session_state = base_state;
        let create_err = apply(
            &mut new_session_state,
            Action::Create {
                buyer_authorized: true,
                cap: 1,
            },
        );
        assert_eq!(create_err, Err(Rejection::UnsupportedAsset));

        // Terminal exit: Settle succeeds on existing session
        let mut settle_state = base_state;
        assert!(apply(
            &mut settle_state,
            Action::Settle {
                seller_authorized: true,
                amount: 1,
            }
        )
        .is_ok());
        assert_eq!(settle_state.status, Status::Settled);
        assert_eq!(settle_state.liability, 0);

        // Terminal exit: Cancel succeeds on existing session
        let mut cancel_state = base_state;
        assert!(apply(
            &mut cancel_state,
            Action::Cancel {
                buyer_authorized: true,
            }
        )
        .is_ok());
        assert_eq!(cancel_state.status, Status::Cancelled);
        assert_eq!(cancel_state.liability, 0);

        // Terminal exit: Expiry recovery succeeds on existing session
        let mut recover_state = base_state;
        assert!(apply(&mut recover_state, Action::AdvanceToExpiry).is_ok());
        assert!(apply(&mut recover_state, Action::Recover).is_ok());
        assert_eq!(recover_state.status, Status::Expired);
        assert_eq!(recover_state.liability, 0);
    }

    #[test]
    fn counterexample_removal_impossible_until_liability_zero() {
        let mut state = State::default();
        apply(
            &mut state,
            Action::Initialize {
                admin_authorized: true,
            },
        )
        .unwrap();

        // Cannot remove active asset
        assert_eq!(
            apply(&mut state, Action::RemoveAsset { caller: 1 }),
            Err(Rejection::AssetNotDeprecated)
        );

        // Open session creates liability
        apply(
            &mut state,
            Action::Create {
                buyer_authorized: true,
                cap: 2,
            },
        )
        .unwrap();
        assert_eq!(state.liability, 2);

        // Deprecate asset
        apply(&mut state, Action::DeprecateAsset { caller: 1 }).unwrap();

        // Removal fails while liability is outstanding
        assert_eq!(
            apply(&mut state, Action::RemoveAsset { caller: 1 }),
            Err(Rejection::AssetLiabilityOutstanding)
        );

        // Settle session to clear liability
        apply(
            &mut state,
            Action::Settle {
                seller_authorized: true,
                amount: 2,
            },
        )
        .unwrap();
        assert_eq!(state.liability, 0);

        // Removal now succeeds
        assert!(apply(&mut state, Action::RemoveAsset { caller: 1 }).is_ok());
        assert_eq!(state.asset_state, AssetState::Absent);
    }

    #[test]
    fn counterexample_only_accepted_nonexpired_rotation_changes_admin() {
        let mut state = State::default();
        apply(
            &mut state,
            Action::Initialize {
                admin_authorized: true,
            },
        )
        .unwrap();
        assert_eq!(state.admin, 1);

        // Propose rotation to admin 2 expiring at ledger 2
        apply(
            &mut state,
            Action::ProposeAdmin {
                caller: 1,
                pending: 2,
                expires_at: 2,
            },
        )
        .unwrap();
        assert_eq!(state.pending_admin, Some(2));
        assert_eq!(state.admin, 1);

        // Cannot overwrite active proposal
        assert_eq!(
            apply(
                &mut state,
                Action::ProposeAdmin {
                    caller: 1,
                    pending: 3,
                    expires_at: 2,
                }
            ),
            Err(Rejection::AdminRotationPending)
        );

        // Unauthorized caller cannot accept
        assert_eq!(
            apply(&mut state, Action::AcceptAdmin { caller: 3 }),
            Err(Rejection::Unauthorized)
        );
        assert_eq!(state.admin, 1);

        // Current admin cannot accept (must be pending)
        assert_eq!(
            apply(&mut state, Action::AcceptAdmin { caller: 1 }),
            Err(Rejection::Unauthorized)
        );
        assert_eq!(state.admin, 1);

        // Admin cancels rotation
        apply(&mut state, Action::CancelAdminRotation { caller: 1 }).unwrap();
        assert_eq!(state.pending_admin, None);
        assert_eq!(state.admin, 1);

        // Propose again with expiry at ledger 2
        apply(
            &mut state,
            Action::ProposeAdmin {
                caller: 1,
                pending: 2,
                expires_at: 2,
            },
        )
        .unwrap();

        // Advance past expiry
        apply(&mut state, Action::AdvanceToExpiry).unwrap();
        assert_eq!(state.ledger, EXPIRY_LEDGER);

        // Expired acceptance fails
        assert_eq!(
            apply(&mut state, Action::AcceptAdmin { caller: 2 }),
            Err(Rejection::AdminRotationExpired)
        );
        assert_eq!(state.admin, 1);

        // Cancel expired rotation
        apply(&mut state, Action::CancelAdminRotation { caller: 1 }).unwrap();

        // Propose with valid future expiry (expires_at = 3 > ledger 2)
        apply(
            &mut state,
            Action::ProposeAdmin {
                caller: 1,
                pending: 2,
                expires_at: 3,
            },
        )
        .unwrap();

        // Pending admin accepts before expiry
        apply(&mut state, Action::AcceptAdmin { caller: 2 }).unwrap();
        assert_eq!(state.admin, 2);
        assert_eq!(state.pending_admin, None);

        // Old admin 1 can no longer propose
        assert_eq!(
            apply(
                &mut state,
                Action::ProposeAdmin {
                    caller: 1,
                    pending: 3,
                    expires_at: 4,
                }
            ),
            Err(Rejection::Unauthorized)
        );

        // New admin 2 can propose
        assert!(apply(
            &mut state,
            Action::ProposeAdmin {
                caller: 2,
                pending: 3,
                expires_at: 4,
            }
        )
        .is_ok());
    }
}
