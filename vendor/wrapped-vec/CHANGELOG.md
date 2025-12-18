Version 0.3.0
=============

    * Updated to 2018 edition of Rust
    * Updated dependencies 

Version 0.2.1
=============

    * Add ability to specify derived traits on generated collections using
      `CollectionDerives` attribute (Issue #3)

Version 0.2.0
=============

    * Complete rewrite as a proc-macro
    * Documentation for generated collection, plus `new`, `iter`, `is_empty` &
     `len` methods on the collection:
        - Attributes for generating custom documentation, if needed
        - Auto-generated default documentation if attrs not given

Version 0.1.2
=============

    * Remove unnecessary outer wrapper macro, fixes issue where wrapped_vec!
      macro would fail when imported from extern crate

Version 0.1.1
=============

    * Add all recommended metadata to Cargo.toml
    * Remove example module in lib.rs

Version 0.1.0
=============

    * Proof of concept macro that generates a wrapped Vec type and associated
      `Iter`, `FromIter` & `Extend` impls, plus a small number of helper
      methods