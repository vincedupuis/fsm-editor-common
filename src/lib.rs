//! The file half of FSM Editor's three editions: reading and writing a machine's two files, converting old (XMI)
//! files, validation, the text grammar, comparing and merging, code generation (through fsm-cli). Every function
//! takes texts and gives texts and data: nothing here reads or writes a file.

pub mod convert;
pub mod format;
pub mod grammar;
pub mod model;
pub mod out;
pub mod paths;
pub mod validate;
pub mod xmi;
pub mod xml;
