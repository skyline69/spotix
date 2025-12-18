wrapped-vec
===========
[![Latest Version](https://img.shields.io/crates/v/wrapped-vec.svg)](https://crates.io/crates/wrapped-vec)
[![Build Status](https://travis-ci.org/bobbo/wrapped-vec.svg?branch=master)](https://travis-ci.org/bobbo/wrapped-vec)

*wrapped-vec* is a Rust crate for auto-generating type definitions and boilerplate code for wrapping 
Vectors in a custom type. It exports a `WrappedVec` custom-derive proc-macro that generates a named
wrapper over `Vec` for any type. For example:

``` rust
#[derive(WrappedVec)]
#[CollectionName="ExampleCollection"]
pub struct ExampleType { ... };
```

will generate 

``` rust
pub struct ExampleCollection(Vec<ExampleType>)
```

A large number of useful trait impls are auto-generated, including `Iter`, `IntoIter` & `Expand`, plus a small
number of useful `Vec`-style methods like `len()`, `iter()` & `is_empty()`.

`WrappedVec` helps you avoid exposing library implementation details or creating brittle APIs that break when 
plain `Vec` doesn't provide the right functionality any more. Type synonyms give collections a custom name
but don't address these issues. The common workaround of simply wrapping `Vec` with a custom type requires
manually implementing common useful collection traits such as `Iter`, which involves a lot of boilerplate. 
Implementing `Deref` targetting `Vec` provides the basic `Vec` methods, but still requires manual implementation
of collection traits.

## Usage

Add `wrapped-vec` to your `Cargo.toml`:

```
wrapped-vec = "0.2"
```

Import the crate with macros:

```
#[macro_use]
use wrapped_vec;
```

Then derive your custom collection and use just like a plain `Vec`:

``` rust
#[derive(WrappedVec)]
#[CollectionName="TaskBatch"]
pub struct Task { ... };

let batch = TaskBatch::from_iter(vec![Task(), Task()]);
for task in batch {
    task.doWork()
}
```

### Generated Type Documentation

`WrappedVec` automatically generates documentation for the derived `Vec` type and the methods 
implemented on it. However, you may wish to override the automated documentation, which can
be done with custom attributes:

``` rust
#[derive(WrappedVec)]
#[CollectionName="TaskBatch"]
#[CollectionDoc="A batch of tasks to be run either in serial or parallel by a TaskRunner"]
pub struct Task { ... };
```

is roughly equivalent to 

``` rust
/// A batch of tasks to be run either in serial or parallel by a TaskRunner
pub struct TaskBatch(Vec<Task>);
```

Documentation attributes available are:

|Attribute|Documents|
|---------|---------|
|`CollectionDoc`|`struct CollectionName`|
|`CollectionNewDoc`|`CollectionName::new`|
|`CollectionLenDoc`|`CollectionName::len`|
|`CollectionIsEmptyDoc`|`CollectionName::is_empty`
|`CollectionIterDoc`|`CollectionName::iter`|

Documentation for trait methods are auto-populated from the parent trait documentation and not
currently overridable.

### Deriving Traits for Generated Collections

The `CollectionDerives` attribute can be used to specify traits which will be derived on the
generated collection type. Traits to be derived are specified as a comma-separated list in a
string. Omitting the `CollectionDerives` attribute, or passing an empty string, causes no trait
derivations to be generated.

``` rust
#[derive(Clone, Debug, WrappedVec)]
#[CollectionName="TaskBatch"]
#[CollectionDerives="Clone, Debug"]
pub struct Task { ... };
```

will generate

``` rust
#[derive(Clone, Debug)]
pub struct TaskBatch(Vec<Task>);
```

## License

Licensed under either of

 * Apache License, Version 2.0, ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
 * MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be dual licensed as above, without any
additional terms or conditions.

