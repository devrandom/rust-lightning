//! A lightweight client for keeping in sync with chain activity.
//!
//! Defines an [`SpvClient`] utility for polling one or more block sources for the best chain tip.
//! It is used to notify listeners of blocks connected or disconnected since the last poll. Useful
//! for keeping a Lightning node in sync with the chain.
//!
//! Defines a [`BlockSource`] trait, which is an asynchronous interface for retrieving block headers
//! and data.
//!
//! Enabling feature `rest-client` or `rpc-client` allows configuring the client to fetch blocks
//! using Bitcoin Core's REST or RPC interface, respectively.
//!
//! Both features support either blocking I/O using `std::net::TcpStream` or, with feature `tokio`,
//! non-blocking I/O using `tokio::net::TcpStream` from inside a Tokio runtime.
//!
//! [`SpvClient`]: struct.SpvClient.html
//! [`BlockSource`]: trait.BlockSource.html

#[cfg(any(feature = "rest-client", feature = "rpc-client"))]
pub mod http;

pub mod poll;

#[cfg(feature = "rest-client")]
pub mod rest;

#[cfg(feature = "rpc-client")]
pub mod rpc;

#[cfg(any(feature = "rest-client", feature = "rpc-client"))]
mod convert;

#[cfg(test)]
mod test_utils;

#[cfg(any(feature = "rest-client", feature = "rpc-client"))]
mod utils;

use crate::poll::{ChainTip, Poll, ValidatedBlockHeader};

use bitcoin::blockdata::block::{Block, BlockHeader};
use bitcoin::hash_types::BlockHash;
use bitcoin::util::uint::Uint256;

use std::future::Future;
use std::pin::Pin;

/// Abstract type for retrieving block headers and data.
pub trait BlockSource : Sync + Send {
	/// Returns the header for a given hash. A height hint may be provided in case a block source
	/// cannot easily find headers based on a hash. This is merely a hint and thus the returned
	/// header must have the same hash as was requested. Otherwise, an error must be returned.
	///
	/// Implementations that cannot find headers based on the hash should return a `Transient` error
	/// when `height_hint` is `None`.
	fn get_header<'a>(&'a mut self, header_hash: &'a BlockHash, height_hint: Option<u32>) -> AsyncBlockSourceResult<'a, BlockHeaderData>;

	/// Returns the block for a given hash. A headers-only block source should return a `Transient`
	/// error.
	fn get_block<'a>(&'a mut self, header_hash: &'a BlockHash) -> AsyncBlockSourceResult<'a, Block>;

	// TODO: Phrase in terms of `Poll` once added.
	/// Returns the hash of the best block and, optionally, its height. When polling a block source,
	/// the height is passed to `get_header` to allow for a more efficient lookup.
	fn get_best_block<'a>(&'a mut self) -> AsyncBlockSourceResult<(BlockHash, Option<u32>)>;
}

/// Result type for `BlockSource` requests.
type BlockSourceResult<T> = Result<T, BlockSourceError>;

// TODO: Replace with BlockSourceResult once `async` trait functions are supported. For details,
// see: https://areweasyncyet.rs.
/// Result type for asynchronous `BlockSource` requests.
type AsyncBlockSourceResult<'a, T> = Pin<Box<dyn Future<Output = BlockSourceResult<T>> + 'a + Send>>;

/// Error type for `BlockSource` requests.
///
/// Transient errors may be resolved when re-polling, but no attempt will be made to re-poll on
/// persistent errors.
#[derive(Debug)]
pub struct BlockSourceError {
	kind: BlockSourceErrorKind,
	error: Box<dyn std::error::Error + Send + Sync>,
}

/// The kind of `BlockSourceError`, either persistent or transient.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BlockSourceErrorKind {
	/// Indicates an error that won't resolve when retrying a request (e.g., invalid data).
	Persistent,

	/// Indicates an error that may resolve when retrying a request (e.g., unresponsive).
	Transient,
}

impl BlockSourceError {
	/// Creates a new persistent error originated from the given error.
	pub fn persistent<E>(error: E) -> Self
	where E: Into<Box<dyn std::error::Error + Send + Sync>> {
		Self {
			kind: BlockSourceErrorKind::Persistent,
			error: error.into(),
		}
	}

	/// Creates a new transient error originated from the given error.
	pub fn transient<E>(error: E) -> Self
	where E: Into<Box<dyn std::error::Error + Send + Sync>> {
		Self {
			kind: BlockSourceErrorKind::Transient,
			error: error.into(),
		}
	}

	/// Returns the kind of error.
	pub fn kind(&self) -> BlockSourceErrorKind {
		self.kind
	}

	/// Converts the error into the underlying error.
	pub fn into_inner(self) -> Box<dyn std::error::Error + Send + Sync> {
		self.error
	}
}

/// A block header and some associated data. This information should be available from most block
/// sources (and, notably, is available in Bitcoin Core's RPC and REST interfaces).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BlockHeaderData {
	/// The block header itself.
	pub header: BlockHeader,

	/// The block height where the genesis block has height 0.
	pub height: u32,

	/// The total chain work in expected number of double-SHA256 hashes required to build a chain
	/// of equivalent weight.
	pub chainwork: Uint256,
}

/// A lightweight client for keeping a listener in sync with the chain, allowing for Simplified
/// Payment Verification (SPV).
///
/// The client is parameterized by a chain poller which is responsible for polling one or more block
/// sources for the best chain tip. During this process it detects any chain forks, determines which
/// constitutes the best chain, and updates the listener accordingly with any blocks that were
/// connected or disconnected since the last poll.
///
/// Block headers for the best chain are maintained in the parameterized cache, allowing for a
/// custom cache eviction policy. This offers flexibility to those sensitive to resource usage.
/// Hence, there is a trade-off between a lower memory footprint and potentially increased network
/// I/O as headers are re-fetched during fork detection.
pub struct SpvClient<P: Poll, C: Cache, L: ChainListener> {
	chain_tip: ValidatedBlockHeader,
	chain_poller: P,
	chain_notifier: ChainNotifier<C>,
	chain_listener: L,
}

/// Adaptor used for notifying when blocks have been connected or disconnected from the chain.
///
/// Used when needing to replay chain data upon startup or as new chain events occur.
pub trait ChainListener {
	/// Notifies the listener that a block was added at the given height.
	fn block_connected(&mut self, block: &Block, height: u32);

	/// Notifies the listener that a block was removed at the given height.
	fn block_disconnected(&mut self, header: &BlockHeader, height: u32);
}

/// The `Cache` trait defines behavior for managing a block header cache, where block headers are
/// keyed by block hash.
///
/// Used by [`ChainNotifier`] to store headers along the best chain. Implementations may define
/// their own cache eviction policy.
///
/// [`ChainNotifier`]: struct.ChainNotifier.html
pub trait Cache {
	/// Retrieves the block header keyed by the given block hash.
	fn get(&self, block_hash: &BlockHash) -> Option<&ValidatedBlockHeader>;

	/// Inserts a block header keyed by the given block hash.
	fn insert(&mut self, block_hash: BlockHash, block_header: ValidatedBlockHeader);

	/// Removes the block header keyed by the given block hash.
	fn remove(&mut self, block_hash: &BlockHash) -> Option<ValidatedBlockHeader>;
}

/// Unbounded cache of block headers keyed by block hash.
pub type UnboundedCache = std::collections::HashMap<BlockHash, ValidatedBlockHeader>;

impl Cache for UnboundedCache {
	fn get(&self, block_hash: &BlockHash) -> Option<&ValidatedBlockHeader> {
		self.get(block_hash)
	}

	fn insert(&mut self, block_hash: BlockHash, block_header: ValidatedBlockHeader) {
		self.insert(block_hash, block_header);
	}

	fn remove(&mut self, block_hash: &BlockHash) -> Option<ValidatedBlockHeader> {
		self.remove(block_hash)
	}
}

impl<P: Poll, C: Cache, L: ChainListener> SpvClient<P, C, L> {
	/// Creates a new SPV client using `chain_tip` as the best known chain tip.
	///
	/// Subsequent calls to [`poll_best_tip`] will poll for the best chain tip using the given chain
	/// poller, which may be configured with one or more block sources to query. At least one block
	/// source must provide headers back from the best chain tip to its common ancestor with
	/// `chain_tip`.
	/// * `header_cache` is used to look up and store headers on the best chain
	/// * `chain_listener` is notified of any blocks connected or disconnected
	///
	/// [`poll_best_tip`]: struct.SpvClient.html#method.poll_best_tip
	pub fn new(
		chain_tip: ValidatedBlockHeader,
		chain_poller: P,
		header_cache: C,
		chain_listener: L,
	) -> Self {
		let chain_notifier = ChainNotifier { header_cache };
		Self { chain_tip, chain_poller, chain_notifier, chain_listener }
	}

	/// Polls for the best tip and updates the chain listener with any connected or disconnected
	/// blocks accordingly.
	///
	/// Returns the best polled chain tip relative to the previous best known tip and whether any
	/// blocks were indeed connected or disconnected.
	pub async fn poll_best_tip(&mut self) -> BlockSourceResult<(ChainTip, bool)> {
		let chain_tip = self.chain_poller.poll_chain_tip(self.chain_tip).await?;
		let blocks_connected = match chain_tip {
			ChainTip::Common => false,
			ChainTip::Better(chain_tip) => {
				debug_assert_ne!(chain_tip.block_hash, self.chain_tip.block_hash);
				debug_assert!(chain_tip.chainwork > self.chain_tip.chainwork);
				self.update_chain_tip(chain_tip).await
			},
			ChainTip::Worse(chain_tip) => {
				debug_assert_ne!(chain_tip.block_hash, self.chain_tip.block_hash);
				debug_assert!(chain_tip.chainwork <= self.chain_tip.chainwork);
				false
			},
		};
		Ok((chain_tip, blocks_connected))
	}

	/// Updates the chain tip, syncing the chain listener with any connected or disconnected
	/// blocks. Returns whether there were any such blocks.
	async fn update_chain_tip(&mut self, best_chain_tip: ValidatedBlockHeader) -> bool {
		match self.chain_notifier.sync_listener(best_chain_tip, &self.chain_tip, &mut self.chain_poller, &mut self.chain_listener).await {
			Ok(_) => {
				self.chain_tip = best_chain_tip;
				true
			},
			Err((_, Some(chain_tip))) if chain_tip.block_hash != self.chain_tip.block_hash => {
				self.chain_tip = chain_tip;
				true
			},
			Err(_) => false,
		}
	}
}

/// Notifies [listeners] of blocks that have been connected or disconnected from the chain.
///
/// [listeners]: trait.ChainListener.html
struct ChainNotifier<C: Cache> {
	/// Cache for looking up headers before fetching from a block source.
	header_cache: C,
}

/// Steps outlining changes needed to be made to the chain in order to transform it from having one
/// chain tip to another.
enum ForkStep {
	ForkPoint(ValidatedBlockHeader),
	DisconnectBlock(ValidatedBlockHeader),
	ConnectBlock(ValidatedBlockHeader),
}

impl<C: Cache> ChainNotifier<C> {
	/// Finds the fork point between `new_header` and `old_header`, disconnecting blocks from
	/// `old_header` to get to that point and then connecting blocks until `new_header`.
	///
	/// Validates headers along the transition path, but doesn't fetch blocks until the chain is
	/// disconnected to the fork point. Thus, this may return an `Err` that includes where the tip
	/// ended up which may not be `new_header`. Note that iff the returned `Err` contains `Some`
	/// header then the transition from `old_header` to `new_header` is valid.
	async fn sync_listener<L: ChainListener, P: Poll>(
		&mut self,
		new_header: ValidatedBlockHeader,
		old_header: &ValidatedBlockHeader,
		chain_poller: &mut P,
		chain_listener: &mut L,
	) -> Result<(), (BlockSourceError, Option<ValidatedBlockHeader>)> {
		let mut events = self.find_fork(new_header, old_header, chain_poller).await.map_err(|e| (e, None))?;

		let mut last_disconnect_tip = None;
		let mut new_tip = None;
		for event in events.iter() {
			match &event {
				&ForkStep::DisconnectBlock(ref header) => {
					println!("Disconnecting block {}", header.block_hash);
					if let Some(cached_header) = self.header_cache.remove(&header.block_hash) {
						assert_eq!(cached_header, *header);
					}
					chain_listener.block_disconnected(&header.header, header.height);
					last_disconnect_tip = Some(header.header.prev_blockhash);
				},
				&ForkStep::ForkPoint(ref header) => {
					new_tip = Some(*header);
				},
				_ => {},
			}
		}

		// If blocks were disconnected, new blocks will connect starting from the fork point.
		// Otherwise, there was no fork, so new blocks connect starting from the old tip.
		assert_eq!(last_disconnect_tip.is_some(), new_tip.is_some());
		if let &Some(ref tip_header) = &new_tip {
			debug_assert_eq!(tip_header.header.block_hash(), *last_disconnect_tip.as_ref().unwrap());
		} else {
			new_tip = Some(*old_header);
		}

		for event in events.drain(..).rev() {
			if let ForkStep::ConnectBlock(header) = event {
				let block = chain_poller
					.fetch_block(&header).await
					.or_else(|e| Err((e, new_tip)))?;
				debug_assert_eq!(block.block_hash, header.block_hash);

				println!("Connecting block {}", header.block_hash);
				self.header_cache.insert(header.block_hash, header);
				chain_listener.block_connected(&block, header.height);
				new_tip = Some(header);
			}
		}
		Ok(())
	}

	/// Walks backwards from `current_header` and `prev_header`, finding the common ancestor.
	/// Returns the steps needed to produce the chain with `current_header` as its tip from the
	/// chain with `prev_header` as its tip. There is no ordering guarantee between different
	/// `ForkStep` types, but `DisconnectBlock` and `ConnectBlock` are each returned in
	/// height-descending order.
	async fn find_fork<P: Poll>(
		&self,
		current_header: ValidatedBlockHeader,
		prev_header: &ValidatedBlockHeader,
		chain_poller: &mut P,
	) -> BlockSourceResult<Vec<ForkStep>> {
		let mut steps = Vec::new();
		let mut current = current_header;
		let mut previous = *prev_header;
		loop {
			// Found the parent block.
			if current.height == previous.height + 1 &&
					current.header.prev_blockhash == previous.block_hash {
				steps.push(ForkStep::ConnectBlock(current));
				break;
			}

			// Found a chain fork.
			if current.header.prev_blockhash == previous.header.prev_blockhash {
				let fork_point = self.look_up_previous_header(chain_poller, &previous).await?;
				steps.push(ForkStep::DisconnectBlock(previous));
				steps.push(ForkStep::ConnectBlock(current));
				steps.push(ForkStep::ForkPoint(fork_point));
				break;
			}

			// Walk back the chain, finding blocks needed to connect and disconnect. Only walk back
			// the header with the greater height, or both if equal heights.
			let current_height = current.height;
			let previous_height = previous.height;
			if current_height <= previous_height {
				steps.push(ForkStep::DisconnectBlock(previous));
				previous = self.look_up_previous_header(chain_poller, &previous).await?;
			}
			if current_height >= previous_height {
				steps.push(ForkStep::ConnectBlock(current));
				current = self.look_up_previous_header(chain_poller, &current).await?;
			}
		}

		Ok(steps)
	}

	/// Returns the previous header for the given header, either by looking it up in the cache or
	/// fetching it if not found.
	async fn look_up_previous_header<P: Poll>(
		&self,
		chain_poller: &mut P,
		header: &ValidatedBlockHeader,
	) -> BlockSourceResult<ValidatedBlockHeader> {
		match self.header_cache.get(&header.header.prev_blockhash) {
			Some(prev_header) => Ok(*prev_header),
			None => chain_poller.look_up_previous_header(header).await,
		}
	}
}

#[cfg(test)]
mod spv_client_tests {
	use crate::test_utils::{Blockchain, NullChainListener};
	use super::*;

	use bitcoin::network::constants::Network;

	#[tokio::test]
	async fn poll_from_chain_without_headers() {
		let mut chain = Blockchain::default().with_height(3).without_headers();
		let best_tip = chain.at_height(1);

		let poller = poll::ChainPoller::new(&mut chain as &mut dyn BlockSource, Network::Testnet);
		let cache = UnboundedCache::new();
		let mut client = SpvClient::new(best_tip, poller, cache, NullChainListener {});
		match client.poll_best_tip().await {
			Err(e) => {
				assert_eq!(e.kind(), BlockSourceErrorKind::Persistent);
				assert_eq!(e.into_inner().as_ref().to_string(), "header not found");
			},
			Ok(_) => panic!("Expected error"),
		}
		assert_eq!(client.chain_tip, best_tip);
	}

	#[tokio::test]
	async fn poll_from_chain_with_common_tip() {
		let mut chain = Blockchain::default().with_height(3);
		let common_tip = chain.tip();

		let poller = poll::ChainPoller::new(&mut chain as &mut dyn BlockSource, Network::Testnet);
		let cache = UnboundedCache::new();
		let mut client = SpvClient::new(common_tip, poller, cache, NullChainListener {});
		match client.poll_best_tip().await {
			Err(e) => panic!("Unexpected error: {:?}", e),
			Ok((chain_tip, blocks_connected)) => {
				assert_eq!(chain_tip, ChainTip::Common);
				assert!(!blocks_connected);
			},
		}
		assert_eq!(client.chain_tip, common_tip);
	}

	#[tokio::test]
	async fn poll_from_chain_with_better_tip() {
		let mut chain = Blockchain::default().with_height(3);
		let new_tip = chain.tip();
		let old_tip = chain.at_height(1);

		let poller = poll::ChainPoller::new(&mut chain as &mut dyn BlockSource, Network::Testnet);
		let cache = UnboundedCache::new();
		let mut client = SpvClient::new(old_tip, poller, cache, NullChainListener {});
		match client.poll_best_tip().await {
			Err(e) => panic!("Unexpected error: {:?}", e),
			Ok((chain_tip, blocks_connected)) => {
				assert_eq!(chain_tip, ChainTip::Better(new_tip));
				assert!(blocks_connected);
			},
		}
		assert_eq!(client.chain_tip, new_tip);
	}

	#[tokio::test]
	async fn poll_from_chain_with_better_tip_and_without_any_new_blocks() {
		let mut chain = Blockchain::default().with_height(3).without_blocks(2..);
		let new_tip = chain.tip();
		let old_tip = chain.at_height(1);

		let poller = poll::ChainPoller::new(&mut chain as &mut dyn BlockSource, Network::Testnet);
		let cache = UnboundedCache::new();
		let mut client = SpvClient::new(old_tip, poller, cache, NullChainListener {});
		match client.poll_best_tip().await {
			Err(e) => panic!("Unexpected error: {:?}", e),
			Ok((chain_tip, blocks_connected)) => {
				assert_eq!(chain_tip, ChainTip::Better(new_tip));
				assert!(!blocks_connected);
			},
		}
		assert_eq!(client.chain_tip, old_tip);
	}

	#[tokio::test]
	async fn poll_from_chain_with_better_tip_and_without_some_new_blocks() {
		let mut chain = Blockchain::default().with_height(3).without_blocks(3..);
		let new_tip = chain.tip();
		let old_tip = chain.at_height(1);

		let poller = poll::ChainPoller::new(&mut chain as &mut dyn BlockSource, Network::Testnet);
		let cache = UnboundedCache::new();
		let mut client = SpvClient::new(old_tip, poller, cache, NullChainListener {});
		match client.poll_best_tip().await {
			Err(e) => panic!("Unexpected error: {:?}", e),
			Ok((chain_tip, blocks_connected)) => {
				assert_eq!(chain_tip, ChainTip::Better(new_tip));
				assert!(blocks_connected);
			},
		}
		assert_eq!(client.chain_tip, chain.at_height(2));
	}

	#[tokio::test]
	async fn poll_from_chain_with_worse_tip() {
		let mut chain = Blockchain::default().with_height(3);
		let best_tip = chain.tip();
		chain.disconnect_tip();
		let worse_tip = chain.tip();

		let poller = poll::ChainPoller::new(&mut chain as &mut dyn BlockSource, Network::Testnet);
		let cache = UnboundedCache::new();
		let mut client = SpvClient::new(best_tip, poller, cache, NullChainListener {});
		match client.poll_best_tip().await {
			Err(e) => panic!("Unexpected error: {:?}", e),
			Ok((chain_tip, blocks_connected)) => {
				assert_eq!(chain_tip, ChainTip::Worse(worse_tip));
				assert!(!blocks_connected);
			},
		}
		assert_eq!(client.chain_tip, best_tip);
	}
}

#[cfg(test)]
mod chain_notifier_tests {
	use crate::test_utils::{Blockchain, MockChainListener};
	use super::*;

	use bitcoin::network::constants::Network;

	#[tokio::test]
	async fn sync_from_same_chain() {
		let mut chain = Blockchain::default().with_height(3);

		let new_tip = chain.tip();
		let old_tip = chain.at_height(1);
		let mut listener = MockChainListener::new()
			.expect_block_connected(*chain.at_height(2))
			.expect_block_connected(*new_tip);
		let mut notifier = ChainNotifier { header_cache: chain.header_cache(0..=1) };
		let mut poller = poll::ChainPoller::new(&mut chain as &mut dyn BlockSource, Network::Testnet);
		match notifier.sync_listener(new_tip, &old_tip, &mut poller, &mut listener).await {
			Err((e, _)) => panic!("Unexpected error: {:?}", e),
			Ok(_) => {},
		}
	}

	#[tokio::test]
	async fn sync_from_different_chains() {
		let mut test_chain = Blockchain::with_network(Network::Testnet).with_height(1);
		let main_chain = Blockchain::with_network(Network::Bitcoin).with_height(1);

		let new_tip = test_chain.tip();
		let old_tip = main_chain.tip();
		let mut listener = MockChainListener::new();
		let mut notifier = ChainNotifier { header_cache: main_chain.header_cache(0..=1) };
		let mut poller = poll::ChainPoller::new(&mut test_chain as &mut dyn BlockSource, Network::Testnet);
		match notifier.sync_listener(new_tip, &old_tip, &mut poller, &mut listener).await {
			Err((e, _)) => {
				assert_eq!(e.kind(), BlockSourceErrorKind::Persistent);
				assert_eq!(e.into_inner().as_ref().to_string(), "genesis block reached");
			},
			Ok(_) => panic!("Expected error"),
		}
	}

	#[tokio::test]
	async fn sync_from_equal_length_fork() {
		let main_chain = Blockchain::default().with_height(2);
		let mut fork_chain = main_chain.fork_at_height(1);

		let new_tip = fork_chain.tip();
		let old_tip = main_chain.tip();
		let mut listener = MockChainListener::new()
			.expect_block_disconnected(*old_tip)
			.expect_block_connected(*new_tip);
		let mut notifier = ChainNotifier { header_cache: main_chain.header_cache(0..=2) };
		let mut poller = poll::ChainPoller::new(&mut fork_chain as &mut dyn BlockSource, Network::Testnet);
		match notifier.sync_listener(new_tip, &old_tip, &mut poller, &mut listener).await {
			Err((e, _)) => panic!("Unexpected error: {:?}", e),
			Ok(_) => {},
		}
	}

	#[tokio::test]
	async fn sync_from_shorter_fork() {
		let main_chain = Blockchain::default().with_height(3);
		let mut fork_chain = main_chain.fork_at_height(1);
		fork_chain.disconnect_tip();

		let new_tip = fork_chain.tip();
		let old_tip = main_chain.tip();
		let mut listener = MockChainListener::new()
			.expect_block_disconnected(*old_tip)
			.expect_block_disconnected(*main_chain.at_height(2))
			.expect_block_connected(*new_tip);
		let mut notifier = ChainNotifier { header_cache: main_chain.header_cache(0..=3) };
		let mut poller = poll::ChainPoller::new(&mut fork_chain as &mut dyn BlockSource, Network::Testnet);
		match notifier.sync_listener(new_tip, &old_tip, &mut poller, &mut listener).await {
			Err((e, _)) => panic!("Unexpected error: {:?}", e),
			Ok(_) => {},
		}
	}

	#[tokio::test]
	async fn sync_from_longer_fork() {
		let mut main_chain = Blockchain::default().with_height(3);
		let mut fork_chain = main_chain.fork_at_height(1);
		main_chain.disconnect_tip();

		let new_tip = fork_chain.tip();
		let old_tip = main_chain.tip();
		let mut listener = MockChainListener::new()
			.expect_block_disconnected(*old_tip)
			.expect_block_connected(*fork_chain.at_height(2))
			.expect_block_connected(*new_tip);
		let mut notifier = ChainNotifier { header_cache: main_chain.header_cache(0..=2) };
		let mut poller = poll::ChainPoller::new(&mut fork_chain as &mut dyn BlockSource, Network::Testnet);
		match notifier.sync_listener(new_tip, &old_tip, &mut poller, &mut listener).await {
			Err((e, _)) => panic!("Unexpected error: {:?}", e),
			Ok(_) => {},
		}
	}

	#[tokio::test]
	async fn sync_from_chain_without_headers() {
		let mut chain = Blockchain::default().with_height(3).without_headers();

		let new_tip = chain.tip();
		let old_tip = chain.at_height(1);
		let mut listener = MockChainListener::new();
		let mut notifier = ChainNotifier { header_cache: chain.header_cache(0..=1) };
		let mut poller = poll::ChainPoller::new(&mut chain as &mut dyn BlockSource, Network::Testnet);
		match notifier.sync_listener(new_tip, &old_tip, &mut poller, &mut listener).await {
			Err((_, tip)) => assert_eq!(tip, None),
			Ok(_) => panic!("Expected error"),
		}
	}

	#[tokio::test]
	async fn sync_from_chain_without_any_new_blocks() {
		let mut chain = Blockchain::default().with_height(3).without_blocks(2..);

		let new_tip = chain.tip();
		let old_tip = chain.at_height(1);
		let mut listener = MockChainListener::new();
		let mut notifier = ChainNotifier { header_cache: chain.header_cache(0..=3) };
		let mut poller = poll::ChainPoller::new(&mut chain as &mut dyn BlockSource, Network::Testnet);
		match notifier.sync_listener(new_tip, &old_tip, &mut poller, &mut listener).await {
			Err((_, tip)) => assert_eq!(tip, Some(old_tip)),
			Ok(_) => panic!("Expected error"),
		}
	}

	#[tokio::test]
	async fn sync_from_chain_without_some_new_blocks() {
		let mut chain = Blockchain::default().with_height(3).without_blocks(3..);

		let new_tip = chain.tip();
		let old_tip = chain.at_height(1);
		let mut listener = MockChainListener::new()
			.expect_block_connected(*chain.at_height(2));
		let mut notifier = ChainNotifier { header_cache: chain.header_cache(0..=3) };
		let mut poller = poll::ChainPoller::new(&mut chain as &mut dyn BlockSource, Network::Testnet);
		match notifier.sync_listener(new_tip, &old_tip, &mut poller, &mut listener).await {
			Err((_, tip)) => assert_eq!(tip, Some(chain.at_height(2))),
			Ok(_) => panic!("Expected error"),
		}
	}
}
