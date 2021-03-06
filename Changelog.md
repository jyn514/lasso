# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## 0.2.4

### Added

- Added Serde support with the `serialize` feature

## 0.2.2

### Fixed

- Fixed `Send` for `Rodeo`

## 0.2.0

### Added

- Added single-threaded interner
- Added `try_get_or_intern`
- Added feature for `hashbrown`
- Added feature for `parking_lot`
- Added `no-std` feature
- Added `Key::try_from_usize`
- Added `MiniSpur`
- Added `MicroCord`
- Removed blanket impls for `u8`-`usize` & the nonzero  varieties
- Added lifetimes to all `Rodeo` types
- Added lifetime to `Key`
- Added the ID requirement to `Key`
- Added `try_resolve`s and `resolve_unchecked`s
- Added `strings()` and `iter()` methods to `Rodeo`, `RodeoResolver` and `RodeoReader`
- Strings are now allocated via an arena allocator

### Changed

- Renamed `Lasso` to `ThreadedRodeo`
- Renamed `ReadOnlyLasso` to `RodeoReader`
- Renamed `ResolverLasso` to `RodeoResolver`
- Changed default impl of `Key::from_usize`
- Added `Send` and `Sync` bounds for `ThreadedRodeo`, `RodeoResolver` and `RodeoReader`
- Changed internals of `get_or_intern` to be `try_get_or_intern.expect()`
- `multi-threaded` is now actually disabled by default

### Removed

- Removed `.intern` from all structs
- `Rodeo` and `ThreadedRodeo` no longer implement `Clone`

### Fixed

- Fixed memory leaks and possible unsoundness
- Fixed data races on `ThreadedRodeo`
- Fixed memory leaks in `ThreadedRodeo`, for real this time

## 0.1.2
## 0.1.1
## 0.1.0
