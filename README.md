Rust-Lightning
==============

[![Crate](https://img.shields.io/crates/v/lightning.svg?logo=rust)](https://crates.io/crates/lightning)
[![Documentation](https://img.shields.io/static/v1?logo=read-the-docs&label=docs.rs&message=lightning&color=informational)](https://docs.rs/lightning/)
[![Safety Dance](https://img.shields.io/badge/unsafe-forbidden-success.svg)](https://github.com/rust-secure-code/safety-dance/)

Rust-Lightning is a Bitcoin Lightning library written in Rust. The main crate,
`lightning`, does not handle networking, persistence, or any other I/O. Thus,
it is runtime-agnostic, but users must implement basic networking logic, chain
interactions, and disk storage.

The `lightning-net-tokio` crate implements Lightning networking using the
[Tokio](https://github.com/tokio-rs/tokio) async runtime.

Status
------

The project implements all of the BOLT specifications in the 1.0 spec except
for [channel queries](https://github.com/lightningnetwork/lightning-rfc/blob/master/07-routing-gossip.md#query-messages). The
implementation has pretty good test coverage that is expected to continue to
improve. There are a number of internal refactorings being done now that will
make the code base more welcoming to new contributors. It is also anticipated
that as developers begin using the API, the lessons from that will result in
changes to the API, so any developer using this API at this stage should be prepared
to embrace that. The current state is sufficient for a developer or project to
experiment with it. Recent increased contribution rate to the project is expected
to lead to a high quality, stable, production-worthy implementation in 2020.

Communications for Rust-Lightning and Lightning Development Kit happens through
[LDK slack](http://lightningdevkit.org/).

Design Goal
-----------

The goal is to provide a full-featured but also incredibly flexible lightning
implementation, allowing the user to decide how they wish to use it. With that
in mind, everything should be exposed via simple, composable APIs. The user
should be able to decide whether they wish to use their own threading/execution
models, allowing usage inside of existing library architectures, or allow us to
handle that for them. Same goes with network connections - if the user wishes
to use their own networking stack, they should be able to do so! This all means
that we should provide simple external interfaces which allow the user to drive
all execution, while implementing sample execution drivers that create a
full-featured lightning daemon by default.

For security reasons, do not add new dependencies. Really do not add new
non-optional/non-test/non-library dependencies. Really really do not add
dependencies with dependencies. Do convince Andrew to cut down dependency usage
in rust-bitcoin.


Tagline
-------

*"Rust-Lightning, not Rusty's Lightning!"*

Contributing
------------

Contributors are warmly welcome, see [CONTRIBUTING.md](CONTRIBUTING.md).

Project Architecture
---------------------

For a Rust-Lightning high-level API introduction, see [ARCH.md](ARCH.md).

License is either Apache-2.0 or MIT, at the option of the user (ie dual-license
Apache-2.0 and MIT).
