
**Usage:**

This example demonstrates how to start a main node, spawn a worker node and send messages from the main node to an actor running in the worker node.

```sh
# build the main-node and worker-node binaries.
cargo build

# start the main-node. The main-node will spawn the worker-node.
cargo run --bin main_node

```