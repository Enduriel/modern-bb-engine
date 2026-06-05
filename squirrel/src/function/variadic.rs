//! [`Variadic`]: a trailing "rest" parameter collecting any number of arguments.

/// A variadic tail parameter: collects every trailing argument as a `Vec<T>`.
///
/// Valid only as the final parameter of a registered closure; its presence
/// switches the closure to an "at least N parameters" check. Deliberately does
/// not implement [`SqArg`](super::SqArg), so it can only ever appear in the tail
/// position.
pub struct Variadic<T>(pub Vec<T>);
