# 0.0.4 (October 21, 2019)

### Changed

- Renamed `Slab::remove` to `Slab::take`.

### Added

- Guards that ensure items are not removed while concurrently accessed.
- `Slab::remove` method that marks an item to be removed when the last thread
  accessing it finishes.

### Fixed

- Potential races between two removals.

# 0.0.3 (October 7, 2019)

### Removed

- `len` and `capacity` APIs that had the potential to be racy.

### Fixed

- Potential race between `remove` and `insert`.
- False sharing that could impact performance.

# 0.0.2 (October 3, 2019)

### Fixed

- Compiler error in release mode.

# 0.0.1 (October 2, 2019)

- Initial release
