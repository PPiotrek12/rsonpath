use std::{collections::HashMap, sync::Arc};

use log::trace;
use serde_json::Value;
use smallvec::SmallVec;

use crate::automaton::{
    ArrayTransition, ArrayTransitionLabel, Automaton, MemberTransition, SimpleSlice, State, StateAttributes, StateTable,
};
use crate::string_pattern::StringPattern;
use rsonpath_syntax::str::JsonString;

/// This struct represents a type definition in the subset of JSON Schema relevant for our use case.
///
/// Type name is the key in the $defs object. All type names should be unique.
///
/// "Properties" are the fields defined in "properties" object of the JSON Schema. Each property
/// represents a child sub-document.
///
/// "Additional properties" property should ALWAYS be set to FALSE. If additional properties are allowed,
/// then the resulting automaton is meaningless (or is it? #TODO)
///
/// "Fake type" indicates that this type is created during parsing to represent array transitions
/// Any other JSON Schema keywords or constructs are ignored in this version
/// 
#[derive(Debug)]
struct SchemaType {
    type_name: String,
    properties: Vec<JsonChild>,
    additional_properties: bool,
    fake_type: bool,
}

impl SchemaType {
    fn new(type_name: String, properties: Vec<JsonChild>, additional_properties: bool) -> Self {
        Self {
            type_name,
            properties,
            additional_properties,
            fake_type: false,
        }
    }

    fn fake(type_name: String, properties: Vec<JsonChild>) -> Self {
        Self {
            type_name,
            properties,
            additional_properties: false,
            fake_type: true,
        }
    }
}

/// This enum represents the type of a child property in the JSON Schema. It can be either:
/// - A reference to another type defined in $defs (Type variant)
/// - An array of items, where each item can be either a reference to another type or primitive
/// - A primitive type (string, integer, etc.) that does not have a corresponding type definition in $defs
#[derive(Debug)]
enum ChildType {
    Type(String),
    Array(Vec<ChildType>),
    Primitive,
}

/// This struct represents a child property in the JSON Schema. It consists of a transition label and a type.
#[derive(Debug)]
struct JsonChild {
    label: String,
    child_type: ChildType,
}

/// This enum represents possible errors that can occur during parsing of the JSON Schema. It includes:
/// - Missing required fields (e.g., "type", "properties", "additionalProperties")
/// - Invalid field types (e.g., "type" is not "object", "properties" is not an object)
/// - Errors during JSON parsing (e.g., invalid JSON syntax)
/// - Duplicate type names
/// - References to undefined types
/// - Other errors with a custom message
#[derive(Debug)]
pub enum ParsingError {
    FieldNotFound { field: String },
    InvalidType { field: String, expected: String },
    InvalidJson(serde_json::Error),
    TypeNotFound { type_name: String, reference: String },
    DuplicateType { type_name: String },
    Error(String),
}

impl ParsingError {
    fn not_found(field: &str) -> Self {
        Self::FieldNotFound {
            field: field.to_string(),
        }
    }

    fn invalid_type(field: &str, expected: &str) -> Self {
        Self::InvalidType {
            field: field.to_string(),
            expected: expected.to_string(),
        }
    }

    fn invalid_json(e: serde_json::Error) -> Self {
        Self::InvalidJson(e)
    }

    fn type_not_found(type_name: &str, reference: &str) -> Self {
        Self::TypeNotFound {
            type_name: type_name.to_string(),
            reference: reference.to_string(),
        }
    }

    fn duplicate_type(type_name: &str) -> Self {
        Self::DuplicateType {
            type_name: type_name.to_string(),
        }
    }

    fn error(message: String) -> Self {
        Self::Error(message)
    }
}

/// Runs the JSON Schema Parser (parse_json_schema) on a given file path.
#[inline]
pub fn from_file(path: &str) -> Result<Automaton, ParsingError> {
    match std::fs::read_to_string(path) {
        Ok(content) => from_string(&content),
        Err(e) => Err(ParsingError::error(format!(
            "IOError: Failed to read file '{}': {}",
            path, e
        ))),
    }
}

/// Runs the JSON Schema Parser (parse_json_schema) on a given JSON schema string.
#[inline]
pub fn from_string(json_schema: &str) -> Result<Automaton, ParsingError> {
    parse_json_schema(json_schema)
}

/// Zwraca nazwę typu wyekstraktowaną z referencji JSON Schema
///
/// # Arguments
/// * `reference` - ścieżka referencji w formacie "#/$defs/TypeName"
///
/// # Example
/// ```
/// let type_name = get_type_name_from_ref("#/$defs/Book");
/// assert_eq!(type_name, "Book");
/// ```
fn get_type_name_from_ref(reference: &str) -> &str {
    if let Some(pos) = reference.rfind('/') {
        return &reference[pos + 1..];
    }
    reference
}

/// Extracts a single ChildType from a JSON value containing a "$ref" definition.
///
/// # Arguments
/// * `value` - The JSON value that should contain a string reference (e.g., "#/$defs/Book")
///
/// # Returns
/// A ChildType::Type variant with the extracted type name, or a ParsingError if the value is not a string
fn extract_type_from_ref(value: &Value) -> Result<ChildType, ParsingError> {
    let reference = value
        .as_str()
        .ok_or_else(|| ParsingError::invalid_type("$ref", "string"))?;
    let type_name = get_type_name_from_ref(reference);
    Ok(ChildType::Type(type_name.to_string()))
}

/// Unpacks the "items" definition of an object with "type": "array".
///
/// As arrays can have heterogenous item types, we handle array as an [*] transition to (possibly) multiple types.
/// # Arguments
/// * `items` - the "items" definition from the JSON Schema of a certain object (sub-document)
///
/// # Returns
/// A vector of ChildType structs representing the possible types of items in the array.
///
/// # Errors
/// Returns ParsingError if the items definition is invalid or missing required fields
fn unpack_array(items: &Value) -> Result<Vec<ChildType>, ParsingError> {
    if let Some(ref_value) = items.get("$ref") {
        return extract_type_from_ref(ref_value).map(|t| vec![t]);
    }
    if items.get("type").is_some() {
        return Ok(vec![ChildType::Primitive]);
    }

    let array = items
        .get("anyOf")
        .ok_or_else(|| ParsingError::not_found("anyOf"))?
        .as_array()
        .ok_or_else(|| ParsingError::invalid_type("anyOf", "array"))?;

    let mut extracted_types = Vec::new();
    for item in array {
        let child_type = if let Some(ref_value) = item.get("$ref") {
            extract_type_from_ref(ref_value)?
        } else if item.get("type").is_some() {
            ChildType::Primitive
        } else {
            return Err(ParsingError::invalid_type("anyOf item", "object with $ref or type"));
        };
        extracted_types.push(child_type);
    }

    Ok(extracted_types)
}

/// Unpacks the "properties" object of a JSON Schema type definition into a vector of JsonChild structs.
///
/// # Arguments
/// * `properties` - the "properties" object from a JSON Schema type definition, represented as a serde_json::Map<String, Value>
///
/// # Returns
/// A vector of JsonChild structs representing the child properties defined in the JSON Schema, or a
/// ParsingError if the properties definition is invalid or missing required fields
fn unpack_properties(properties: &serde_json::Map<String, Value>) -> Result<Vec<JsonChild>, ParsingError> {
    let mut children = Vec::new();

    trace!("Unpacking properties: {:?}", properties);

    for (label, child_def) in properties {
        let child: JsonChild = if let Ok(reference) = get_required_string_field(child_def, "$ref") {
            let type_name = get_type_name_from_ref(reference);
            JsonChild {
                label: label.clone(),
                child_type: ChildType::Type(type_name.to_string()),
            }
        } else if let Some(items) = child_def.get("items") {
            let extracted_types: Vec<ChildType> = unpack_array(items)?;
            JsonChild {
                label: label.clone(),
                child_type: ChildType::Array(extracted_types),
            }
        } else {
            JsonChild {
                label: label.clone(),
                child_type: ChildType::Primitive,
            }
        };

        children.push(child);
    }

    Ok(children)
}

/// Processes a single type definition from the JSON Schema and constructs a SchemaType struct.
/// Unpacks the required fields ("type", "additionalProperties", "properties") and validates their values.
///
/// # Arguments
/// * `definition` - a tuple containing the type name (key in $defs) and
///   the corresponding JSON value representing the type definition (object with "type", "properties", etc.)
fn process_type(definition: (&String, &Value)) -> Result<SchemaType, ParsingError> {
    let (type_name, type_def) = definition;

    if let Some(type_value) = type_def.get("type") {
        if type_value != "object" {
            return Err(ParsingError::invalid_type(type_name, "object"));
        }
    } else {
        return Err(ParsingError::not_found(&format!("type for '{}'", type_name)));
    }

    if let Some(additional_properties) = type_def.get("additionalProperties") {
        if additional_properties != &Value::Bool(false) {
            return Err(ParsingError::invalid_type(
                type_name,
                "additionalProperties set to false",
            ));
        }
    } else {
        return Err(ParsingError::not_found(&format!(
            "additionalProperties for '{}'",
            type_name
        )));
    }

    trace!("Processing type: {:?}", type_name);
    let properties = get_required_object_field(type_def, "properties")?;

    let child_map = unpack_properties(properties)?;

    Ok(SchemaType::new(type_name.clone(), child_map, false))
}

/// This function creates a JsonChild for Arrays.
fn new_array_child(item_type: &ChildType) -> JsonChild {
    let array_child_type = match item_type {
        ChildType::Type(t) => ChildType::Type(t.clone()),
        ChildType::Primitive => ChildType::Primitive,
        ChildType::Array(_) => unreachable!("Nested arrays should be caught during parsing"),
    };
    JsonChild {
        label: String::new(),
        child_type: array_child_type,
    }
}

/// Creates synthetic SchemaType instances for array types to handle array transitions in the automaton
///
/// This function consumes initial type definitions, because those are a half-way product.
fn unroll_arrays(types: Vec<SchemaType>) -> Result<Vec<SchemaType>, ParsingError> {
    let mut unrolled_types = Vec::new();

    for t in types {
        let mut children = Vec::new();

        for child in t.properties {
            if let ChildType::Array(item_types) = &child.child_type {
                let array_type_name = format!("{}{}[]", t.type_name, child.label);

                let properties: Vec<JsonChild> = item_types.iter().map(new_array_child).collect();

                let array_schema_type = SchemaType::fake(array_type_name.clone(), properties);

                unrolled_types.push(array_schema_type);

                children.push(JsonChild {
                    label: child.label.clone(),
                    child_type: ChildType::Type(array_type_name),
                });
            } else {
                children.push(child);
            }
        }

        unrolled_types.push(SchemaType::new(t.type_name, children, t.additional_properties));
    }

    Ok(unrolled_types)
}

/// Helper funtion to create a transition with a string label
fn new_member_transition(label: &str, target: State) -> MemberTransition {
    let json_string = JsonString::new(label);
    let pattern = StringPattern::new(&json_string);
    (Arc::new(pattern), target)
}

/// Helper function to create an array transition with wildcard slice [*]
fn new_array_transition(target: State) -> ArrayTransition {
    use rsonpath_syntax::num::JsonUInt;
    let slice = SimpleSlice::new(JsonUInt::ZERO, None, JsonUInt::ONE);
    ArrayTransition::new(ArrayTransitionLabel::Slice(slice), target)
}

/// Constructs an automaton from a list of JSON Type definitions (SchemaType).
/// Each type corresponds to a state in the automaton, and the properties of the type define the transitions to other states.
///
/// # Arguments
/// * `types` - a slice of SchemaType structs representing the types defined in the JSON Schema after processing and unrolling arrays.
///
/// # Returns
/// An Automaton representing the paths defined by the JSON Schema, or a ParsingError
fn construct_automaton_from_types(types: &[SchemaType]) -> Result<Automaton, ParsingError> {
    let mut states: Vec<StateTable> = vec![
        StateTable::new(
            // LEAF state
            StateAttributes::ACCEPTING,
            SmallVec::new(),
            SmallVec::new(),
            crate::automaton::State::new(0),
        ),
        StateTable::new(
            // REJECT state
            StateAttributes::REJECTING,
            SmallVec::new(),
            SmallVec::new(),
            crate::automaton::State::new(1),
        ),
    ];

    let mut counter: u8 = 2;
    let mut type_to_state: HashMap<String, u8> = HashMap::new();

    for t in types {
        type_to_state.insert(t.type_name.clone(), counter);
        counter += 1;
    }

    for t in types {
        let mut member_transitions: SmallVec<[MemberTransition; 2]> = SmallVec::new();
        let mut array_transitions: SmallVec<[ArrayTransition; 2]> = SmallVec::new();

        for child in &t.properties {
            let child_idx: u8 = match &child.child_type {
                ChildType::Type(type_name) => *type_to_state
                    .get(type_name)
                    .ok_or_else(|| ParsingError::type_not_found(&t.type_name, type_name))?,
                ChildType::Primitive => 0,
                ChildType::Array(_) => {
                    unreachable!("Arrays should have been unrolled into separate types during parsing")
                }
            };

            if t.fake_type {
                array_transitions.push(new_array_transition(State::new(child_idx)));
            } else {
                member_transitions.push(new_member_transition(&child.label, State::new(child_idx)));
            }
        }

        states.push(StateTable::new(
            StateAttributes::ACCEPTING,
            member_transitions,
            array_transitions,
            crate::automaton::State::new(1),
        ));
    }

    Ok(Automaton::from_states(states))
}

/// Extracts a required field from a JSON value and ensures it is an object.
fn get_required_object_field<'a>(
    value: &'a Value,
    field_name: &str,
) -> Result<&'a serde_json::Map<String, Value>, ParsingError> {
    let field_value = value
        .get(field_name)
        .ok_or_else(|| ParsingError::not_found(field_name))?;

    field_value
        .as_object()
        .ok_or_else(|| ParsingError::invalid_type(field_name, "object"))
}

/// Extracts a required field from a JSON value and ensures it is a string.
fn get_required_string_field<'a>(value: &'a Value, field_name: &str) -> Result<&'a str, ParsingError> {
    let field_value = value
        .get(field_name)
        .ok_or_else(|| ParsingError::not_found(field_name))?;

    field_value
        .as_str()
        .ok_or_else(|| ParsingError::invalid_type(field_name, "string"))
}

fn find_duplicate_in_vec(vec: &[String]) -> Option<String> {
    let mut seen = std::collections::HashSet::new();
    for item in vec {
        if !seen.insert(item) {
            return Some(item.clone());
        }
    }
    None
}

/// Parses a JSON Schema from a file and constructs an non-deterministic automaton representing
/// all of the possible paths (labels) that can occur in documents that pass validation against this schema.
///
/// Workflow:
/// 1. Parse the JSON schema string into a serde_json::Value
/// 2. Extract the $defs object and iterate over its entries to create SchemaType instances for each defined type
/// 3. Unroll array types into separate SchemaType instances to simplify automaton construction
/// 4. Construct the automaton states and transitions based on the processed SchemaType instances
fn parse_json_schema(json_schema: &str) -> Result<Automaton, ParsingError> {
    let schema: Value = match serde_json::from_str(json_schema) {
        Ok(s) => s,
        Err(e) => {
            return Err(ParsingError::invalid_json(e));
        }
    };

    let defs = get_required_object_field(&schema, "$defs")?;

    let types = defs
        .iter()
        .map(process_type)
        .collect::<Result<Vec<SchemaType>, ParsingError>>()?;

    let type_names = types.iter().map(|t| t.type_name.clone()).collect::<Vec<String>>();

    if let Some(duplicate) = find_duplicate_in_vec(&type_names) {
        return Err(ParsingError::duplicate_type(&duplicate));
    }

    trace!("Parsed types: {:?}", types);

    let types_with_arrays = unroll_arrays(types)?;

    trace!("Types after unrolling arrays: {:?}", &types_with_arrays);

    construct_automaton_from_types(&types_with_arrays)
}

#[cfg(test)]
mod tests {
    use super::{from_file, from_string};
    use crate::{
        automaton::{ArrayTransition, ArrayTransitionLabel, Automaton, SimpleSlice, State, StateAttributes, StateTable},
        StringPattern,
    };
    use std::sync::Arc;

    #[test]
    fn parses_example_schema_into_expected_automaton() {
        let schema = include_str!("../../examples/example_schema.json");
        let from_string_automaton = from_string(schema).unwrap();
        let from_file_automaton =
            from_file(&format!("{}/examples/example_schema.json", env!("CARGO_MANIFEST_DIR"))).unwrap();
        let expected_automaton = expected_example_schema_automaton();

        println!("from_string:\n{from_string_automaton}");
        println!("from_file:\n{from_file_automaton}");
        println!("expected:\n{expected_automaton}");

        assert_eq!(from_string_automaton, expected_automaton);
        assert_eq!(from_string_automaton, from_file_automaton);
    }

    fn expected_example_schema_automaton() -> Automaton {
        let cast_to_person = ArrayTransition::new(
            ArrayTransitionLabel::Slice(SimpleSlice::new(0.into(), None, 1.into())),
            State::new(5),
        );
        let content_to_item_book = ArrayTransition::new(
            ArrayTransitionLabel::Slice(SimpleSlice::new(0.into(), None, 1.into())),
            State::new(2),
        );
        let content_to_item_movie = ArrayTransition::new(
            ArrayTransitionLabel::Slice(SimpleSlice::new(0.into(), None, 1.into())),
            State::new(4),
        );

        Automaton::from_states(vec![
            state_table(vec![], vec![], State::new(0), StateAttributes::ACCEPTING),
            state_table(vec![], vec![], State::new(1), StateAttributes::REJECTING),
            state_table(
                vec![
                    member("title", State::new(0)),
                    member("author", State::new(5)),
                    member("length", State::new(0)),
                ],
                vec![],
                State::new(1),
                StateAttributes::ACCEPTING,
            ),
            state_table(vec![], vec![cast_to_person], State::new(1), StateAttributes::ACCEPTING),
            state_table(
                vec![
                    member("title", State::new(0)),
                    member("cast", State::new(3)),
                    member("length", State::new(0)),
                ],
                vec![],
                State::new(1),
                StateAttributes::ACCEPTING,
            ),
            state_table(
                vec![member("name", State::new(0)), member("age", State::new(0))],
                vec![],
                State::new(1),
                StateAttributes::ACCEPTING,
            ),
            state_table(
                vec![],
                vec![content_to_item_book, content_to_item_movie],
                State::new(1),
                StateAttributes::ACCEPTING,
            ),
            state_table(
                vec![member("content", State::new(6))],
                vec![],
                State::new(1),
                StateAttributes::ACCEPTING,
            ),
        ])
    }

    fn state_table(
        member_transitions: Vec<(Arc<StringPattern>, State)>,
        array_transitions: Vec<ArrayTransition>,
        fallback_state: State,
        attributes: StateAttributes,
    ) -> StateTable {
        StateTable::new(
            attributes,
            member_transitions.into_iter().collect(),
            array_transitions.into_iter().collect(),
            fallback_state,
        )
    }

    fn member(label: &str, target: State) -> (Arc<StringPattern>, State) {
        let json_string = rsonpath_syntax::str::JsonString::new(label);
        (Arc::new(StringPattern::new(&json_string)), target)
    }
}
