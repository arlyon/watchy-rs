# layered-lock

A `no_std` compatible, layered mutex designed to model recursive dependencies and resource management, particularly in embedded systems. Intermediate layers can represent states like sleep modes or enabling/disabling hardware components (e.g., a modem).

## Overview

The `layered-lock` crate provides a mechanism to manage complex locking scenarios where acquiring a lock might depend on the state of other, potentially hierarchical, resources. It achieves this by modeling a series of locks, one for each "leaf" node, and a set of dependencies between them.

An intermediate lock (or layer) is considered "open" or acquired if all of its dependent leaf nodes are open. Optionally, a layer can prevent its leaf nodes from being opened, which is useful for implementing graceful shutdown procedures.

## Core Concepts

* **`AsyncMutex<T>` Trait**: A generic trait defining the behavior of an asynchronous mutex. This allows `LayeredLock` to be used with different mutex implementations.
* **`LayeredLock<MAX_LEAFS, MAX_CON, M>`**: The main struct representing the layered locking system.
    * `MAX_LEAFS`: The maximum number of leaf locks.
    * `MAX_CON`: The maximum number of connections or dependencies.
    * `M`: The type of `AsyncMutex` used for the individual locks.
    * It stores locks in a `heapless::Vec` and dependencies as offsets within this vector.
* **`Leaf<C, State>`**: Represents a leaf node in the lock hierarchy. Its state can be `Open` (siblings can be added) or `Fused` (no more siblings can be added).
* **`Layer<C>`**: Represents an intermediate layer in the lock hierarchy. It can be locked, which would involve managing its child locks.

## How it Works

The `LayeredLock` is initialized with a single lock and a dependency pointing to an implicit root layer. New locks (leaves) can be added as:
* **Siblings**: `push_sibling` adds a new leaf at the same level as an existing leaf, or relative to a specified layer.
* **Children**: `push_child` adds a new leaf as a child of an existing leaf, effectively creating a new layer.

Dependencies are managed as a list of relative offsets. `find_node` and `find_lock_idx` are utility functions to navigate this dependency tree based on a "path" (an array of sibling counts).

## Features

* **`no_std`**: Suitable for embedded environments where the standard library is not available.
* **Asynchronous**: Designed to work with async runtimes.
* **Recursive Dependency Modeling**: Allows for complex dependency chains between resources.
* **Graceful Shutdown**: Layers can prevent child locks from being acquired.
* **Heapless**: Uses `heapless` collections to avoid dynamic memory allocation.

## API Overview

* LayeredLock::new(): Creates a new layered lock and an initial leaf.
* LayeredLock::push_sibling(): Adds a sibling leaf.
* LayeredLock::push_child(): Adds a child leaf and returns the new layer and leaf.
* Layer::lock(): Asynchronously locks a layer, managing its child locks.
* Layer::try_lock(): Attempts to lock a layer without blocking. (Currently a placeholder in the provided code).
* find_node(): Finds a node in the dependency tree.
* find_lock_idx(): Finds the index of a lock corresponding to a leaf path.

## Limitations and Future Work
* The Layer::lock() and Layer::try_lock() implementations are not fully detailed in the provided lib.rs and would require further development based on the desired locking semantics.
* The child_locks() method seems to be a work in progress.
* Error handling could be expanded (e.g., for exceeding MAX_LEAFS or MAX_CON).

## Contributing

Contributions are welcome! Please feel free to submit issues or pull requests.

## License

This crate is licensed under terms compatible with the parent watchy-rs project (e.g., MIT or Apache 2.0). Please refer to the main project's license.
