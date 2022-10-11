
<h1 align=center> Examples and Benchmark Discussions</h1>

# Demo Examples:

* **example_ask** : A simple example demostrating the messaging ask pattern.
```sh
# run command
cargo run --example example_ask
```

* **example_ipc_remote_ask** : A simple example demostrating the messaging ask pattern in the ipc-cluster configuration.
```sh
# build the main-node and worker-node binaries. directly using cargo run won't build the worker-node.
cd example_ipc_remote_ask
cargo build

# start the main-node. The main-node will spawn the worker-node.
cargo run --bin main_node
``` 

# Benchmark Examples:

Examples added to run benchmarks for performance and memory utilization analysis.

* **example_ring_ask** and **example_ring_test** : Spawn nodes and send messages in a ring.
```sh

# example_ring_tell
cargo build --release
cargo run --example example_ring_tell --release -- --num-rounds 2000 --num-nodes 5000

# example_ring_ask
cargo build --release
cargo run --example example_ring_ask --release -- --num-rounds 2000 --num-nodes 5000

# =======================================================================
# RESULT EXAMPLE_RING_TELL
# System: Corei7-7700HQ CPU @ 2.80GHz (Ubuntu 20.04 wsl2 in Windows 10) + 16GB RAM
# (1) Maximum RAM utilization approx 13 MBs.
# (2) Actix takes approx 2 seconds for 10_000_000 messages in its ring test on the same machine.
#     REF: https://github.com/actix/actix/blob/master/actix/examples/ring.rs
# =======================================================================
num_ring_rounds: 2000
num_nodes: 5000
time_taken_for_node_creation: 22.9965ms
final_sum_after_ring_messaging: 10000000
time_taken_for_ring_messaging_tell: 9.1110963s

# =======================================================================
# RESULT EXAMPLE_RING_ASK
# System: Corei7-7700HQ CPU @ 2.80GHz (Ubuntu 20.04 wsl2 in Windows 10) + 16GB RAM
# (1) Maximum RAM utilization approx 5 GBs.
# (2) We have very high memory usage in case of "ask". Have to profile, but seems the future chains 
#     are adding significant memory costs. Currently, don't have a similar benchmark from Actix, 
#     so made the below naive change to the actix/examples/ring.rs test to replicate the "ask" 
#     pattern in Actix. Memory usage for actix was about 6 GBs. Not making any claims about
#     performance and memory, just wanted to check what is the memory cost of keeping the
#     future-chain alive for 10_000_000 ring-messages.
#
#     ```naive_change_to_actix
#       
#       // self.next.do_send(Payload(msg.0 + 1));
#       let next = self.next.clone();
#       Box::pin(async move { next.send(Payload(msg.0 + 1)).await.unwrap_or(Ok(())) })
#
#     ```
#
# =======================================================================
num_ring_rounds: 2000
num_nodes: 5000
time_taken_for_node_creation: 27.5208ms
final_sum_after_ring_messaging: 10000000
time_taken_for_ring_messaging: 19.1439998s

# =======================================================================
```

* **example_ipc_remote_storm**:

Spawn client nodes those will storm messages to the main_node.
```sh
# build the main-node and worker-node binaries. directly using cargo run won't build the worker-node.
cargo build --release

# start the main-node. The main-node will spawn the worker-nodes.
cargo run --bin main_node --release -- --num-msgs 10000 --num-nodes 100

# =======================================================================
# System: Corei7-7700HQ CPU @ 2.80GHz (Ubuntu 20.04 wsl2 in Windows 10) + 16GB RAM
# TOKIO_RT FOR MAIN_NODE: 4 worker threads. SYSTEM_EXECUTOR: 1 worker thread.
# TOKIO_RT FOR WORKER_NODE: 1 worker thread. SYSTEM_EXECUTOR: 1 worker thread.
# RAM UTILIZATION: MAIN_NODE: approx 12 MB. WORKER_NODE(each): 3.5 MB.
# 
# NOTE: Currently the worker nodes are all sending messages to the main node
# over the same socket. Need to get the results if main-node starts listening
# to individual sockets of the worker nodes.
# =======================================================================
num_msgs: 10000
num_nodes: 100
time_taken_to_spawn_worker_nodes: 177.579ms
time_taken_to_schedule_storm: 245.2796ms
time_taken_to_storm_from_node_id_4_: 34.5481316s
final_sum_from_node_id_4_: 10000
time_taken_to_storm_from_node_id_9_: 34.5513039s
final_sum_from_node_id_9_: 10000
time_taken_to_storm_from_node_id_8_: 34.6276106s
final_sum_from_node_id_8_: 10000
time_taken_to_storm_from_node_id_44_: 34.6343555s
...
...
time_taken_to_storm_from_node_id_98_: 34.8106569s
final_sum_from_node_id_98_: 10000
time_taken_to_storm_from_node_id_92_: 34.849391s
final_sum_from_node_id_92_: 10000
terminating_worker_nodes
main_node_end


```

* **example_ipc_remote_ring**:

This example demonstrates how to start a main node, spawn a number of worker nodes and send ring messages starting from the main node to the worker nodes in a ring and performing a sum operation.

```sh
# build the main-node and worker-node binaries. directly using cargo run won't build the worker-node.
cargo build --release

# start the main-node. The main-node will spawn the worker-nodes.
# send-ask: 0 [TELL] and 1 [ASK]
cargo run --bin main_node --release -- --send-ask 1 --num-rounds 2 --num-nodes 20

# =======================================================================
# Results from both send_ask and send_tell demo runs (ipc-cluster):
# System: Corei7-7700HQ CPU @ 2.80GHz (Ubuntu 20.04 wsl2 in Windows 10) + 16GB RAM
#
# ASK : cargo run --bin main_node --release -- --send-ask 1 --num-rounds 1000 --num-nodes 20
# TELL: cargo run --bin main_node --release -- --send-ask 0 --num-rounds 1000 --num-nodes 20
#
# NOTE: There are some performance issues in our ipc-messaging during the ring-tests.
#       The problem is not CPU/memory bound as CPU utilization is nominal and both
#       main/worker processes are consuming around 4MB RAM. Seems too much time 
#       on pooling and locks. Have to spend some time on profiling and fix this
#       or else this ipc-cluster implementation remains experimental.
# REF: Unix socket ipc-benchmark https://github.com/goldsborough/ipc-bench
# REF: https://www.mpi-hd.mpg.de/personalhomes/fwerner/research/2021/09/grpc-for-ipc/
# =======================================================================

# [ASK]
send_ask: 1
num_ring_rounds: 1000
num_nodes: 5
time_taken_to_spawn_worker_nodes: 10.33ms
ring_complete_final_sum: 5000
time_taken_for_ring_messaging_to_complete: 2.2511585s
terminating_worker_nodes
main_node_end

# [ASK]
send_ask: 1
num_ring_rounds: 500
num_nodes: 20
time_taken_to_spawn_worker_nodes: 47.6948ms
ring_complete_final_sum: 10000
time_taken_for_ring_messaging_to_complete: 4.5020191s
terminating_worker_nodes
main_node_end

# [ASK]
send_ask: 1
num_ring_rounds: 1000
num_nodes: 20
time_taken_to_spawn_worker_nodes: 50.9043ms
ring_complete_final_sum: 20000
time_taken_for_ring_messaging_to_complete: 8.8861631s
terminating_worker_nodes
main_node_end

# [TELL]
send_ask: 0
num_ring_rounds: 1000
num_nodes: 200
time_taken_to_spawn_worker_nodes: 379.0756ms
ring_complete_final_sum: 200000
time_taken_for_ring_messaging_to_complete: 63.5759996s
terminating_worker_nodes
main_node_end

# [TELL]
send_ask: 0
num_ring_rounds: 100000
num_nodes: 2
time_taken_to_spawn_worker_nodes: 3.2669ms
ring_complete_final_sum: 200000
time_taken_for_ring_messaging_to_complete: 67.0055855s
terminating_worker_nodes
main_node_end

# [TELL]
send_ask: 0
num_ring_rounds: 25000
num_nodes: 2
time_taken_to_spawn_worker_nodes: 2.7863ms
ring_complete_final_sum: 50000
time_taken_for_ring_messaging_to_complete: 17.00157s
terminating_worker_nodes
main_node_end

# =======================================================================

```

**Utility commands for performance analysis:**

```sh

# =======================================================================
# IMPORTANT:
# (1) These examples were added to test resource utilization
# and performance. So depending on the number of nodes and rounds
# the examples may panic leaving running workers behind.
# (2) We use tarpc crate for ipc-messaging and have a "10 seconds"
# timeout on rpc-requests, so if the ring/request(s) continues for more than
# 10 seconds it will result in panic as the examples have expect() assertions.
# (3) We will expose the timeout as a config to resolve this issue.
# (4) The timeout comes into play only during the example_ipc_remote_ring  
#     send_ask test not during the send_tell test, as during the send_ask 
#     test the entire future-chain remains in waiting mode, till the last
#     add operation is done and the chain-returns. This pattern
#     serves as a good test for the memory utilization.
#
# List of commands to clean up in case of panics and also to
# monitor resource utilization:
# =======================================================================

# monitor linux system vital resources
htop

# list threads of the system
ps -eLf

# list thread count for a specific process
ps -o thcount <pid>

# list pids of all the running worker processes
pgrep -fi 'worker_node --node-id'

# kill all the running worker processes
kill -9 $(pgrep -fi 'worker_node --node-id')

```

**Build and Run `perf` in Ubuntu 20.04 running on Windows 10 WSL2:**

```sh
# get the required build tools
sudo apt install build-essential flex bison libssl-dev libelf-dev

# shallow clone the wsl2 kernel repo and build the perf tools
# REF: https://stackoverflow.com/questions/60237123/is-there-any-method-to-run-perf-under-wsl
mkdir ~/repos/perf
cd ~/repos/perf
git clone --depth=1 https://github.com/microsoft/WSL2-Linux-Kernel.git
cd WSL2-Linux-Kernel/tools/perf
make

# add perf binary path to PATH
export PATH="~/repos/perf/WSL2-Linux-Kernel/tools/perf:$PATH"

# Build the carte in release mode. Add "debug=true" to [profile.release] section in Cargo.toml.
cd /path/to/example_ipc_remote_ring
cargo build --release

# run perf.
mkdir .perf
perf record --output=./.perf/perf.data  --call-graph dwarf ./target/release/main_node --send-ask 0 --num-rounds 25000 --num-nodes 2
perf report --input=./.perf/perf.data --hierarchy -M intel

# run or integrate other tools like flamegraph/inferno/pprof if necessary.
# REF: https://github.com/jonhoo/inferno
# REF: https://github.com/flamegraph-rs/flamegraph
# REF: https://crates.io/crates/pprof

# clone inferno repository, build, and add binaries to path.
mkdir ~/repos/temp.inferno
cd ~/repos/temp.inferno
git clone https://github.com/jonhoo/inferno.git
cd inferno
cargo build --release
export PATH="~/repos/temp.inferno/inferno/target/release:$PATH"

# run inferno to generate flamegraph
perf script --input=./.perf/perf.data | inferno-collapse-perf > ./.perf/stacks.folded
cat ./.perf/stacks.folded | inferno-flamegraph > ./.perf/flamegraph.svg

```