use std::sync::Arc;

use rsonpath_syntax::str::JsonString;

use crate::{
    automaton::{
        ArrayTransition, ArrayTransitionLabel, MemberTransition, SimpleSlice, State, StateAttributes, StateTable,
    },
    StringPattern,
};

/// Helper funtion to create a transition with a string label
pub(crate) fn new_member_transition(label: &str, target: State) -> MemberTransition {
    let json_string = JsonString::new(label);
    let pattern = StringPattern::new(&json_string);
    (Arc::new(pattern), target)
}

/// Helper function to create an array transition with wildcard slice [*]
pub(crate) fn new_array_transition(target: State) -> ArrayTransition {
    use rsonpath_syntax::num::JsonUInt;
    let slice = SimpleSlice::new(JsonUInt::ZERO, None, JsonUInt::ONE);
    ArrayTransition::new(ArrayTransitionLabel::Slice(slice), target)
}

/// Helper that creates an all-rejecting state
pub(crate) fn new_dumpster_state() -> StateTable {
    StateTable::new(StateAttributes::REJECTING, vec![].into(), vec![].into(), State::new(0))
}
