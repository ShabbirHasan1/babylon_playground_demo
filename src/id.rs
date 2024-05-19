use std::{
    sync::atomic::{AtomicU16, AtomicU32, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};
use thiserror::Error;

#[derive(Error, Debug, Copy, Clone, PartialEq, Eq)]
pub enum SmartIdError {
    #[error("Sequence number overflow")]
    SequenceNumberOverflow,
}

/// Instruction ID type. This is a unique identifier for an instruction.
pub type InstructionId = u64;

/// A coin is a symbol on an exchange that can be traded.
/// NOTE: This is a local id, not the id used by the exchange.
pub type CoinId = u32;

/// A pair is a combination of two coins, e.g. BTC/USDT
/// FIXME: Do I want this really?
/// NOTE: This id is a shifted combination of the two coin ids, e.g.: ((base_id as u64) << 32) | (quote_id as u64)
/// NOTE: This is a local id, not the id used by the exchange.
pub type CoinPairId = u64;

// Atomic counters for handle and order IDs.
// These are used to generate unique IDs for handles and orders.
static HANDLE_COUNTER: AtomicU16 = AtomicU16::new(0);
static ORDER_COUNTER: AtomicU16 = AtomicU16::new(0);

// Bit size constants for different components of SmartId.
const HANDLE_ID_BITS: u64 = 16;
const ORDER_KIND_BITS: u64 = 8;
const ORDER_SEQUENCE_NUMBER_BITS: u64 = 8;
const UNIQUE_ORDER_ID_BITS: u64 = 32; // 16 bits for timestamp, 16 bits for counter

// Type aliases for clarity and readability.
pub type OrderId = u64; // Full identifier for an order. Though can be used as just a placeholder for an ID (e.g. remote order ID).
pub type HandleId = u16; // Unique identifier for a handle.
pub type UniqueOrderId = u32; // Unique identifier for an order (within a handle)
pub type OrderSequenceNumber = u8; // Sequence number to track order changes (e.g. for cancelReplace requests)

/// Represents an order's unique identifier.
/// Encoded as a 64-bit integer, combining trade ID, order kind, sequence number, and a unique order ID.
/// Layout: [Trade ID (16 bits)][Order Kind (8 bits)][Sequence Number (8 bits)][Unique Order ID (32 bits)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct SmartId(pub OrderId);

/// Enum representing different types of handles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
#[repr(u8)]
pub enum HandleKind {
    #[default]
    None = 0,
    Trade = 1,
    Position = 2,
}

/// Enum representing different types of orders.
/// Used for encoding the order kind in the `SmartId`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
#[repr(u8)]
pub enum HandleOrderKind {
    #[default]
    None = 0,
    EntryOrder = 1,
    ExitOrder = 2,
    StoplossOrder = 3,
    SpecialOrder = 4,    // Represents the SPECIAL order (e.g. produced by a strategy or Trade)
    StandaloneOrder = 5, // Represents a standalone order
    UnexpectedOrder = 6, // Represents an unexpected order (e.g. created on the exchange without being tracked by the system)
    Unknown = 255,       // Used for unrecognized order types.
}

/// Converts a u8 value into a `TradeOrderKind`.
/// Useful for decoding a `SmartId` into its constituent parts.
impl From<u8> for HandleOrderKind {
    fn from(value: u8) -> Self {
        match value {
            0 => HandleOrderKind::None,
            1 => HandleOrderKind::EntryOrder,
            2 => HandleOrderKind::ExitOrder,
            3 => HandleOrderKind::StoplossOrder,
            4 => HandleOrderKind::SpecialOrder,
            5 => HandleOrderKind::StandaloneOrder,
            6 => HandleOrderKind::UnexpectedOrder,
            _ => HandleOrderKind::Unknown,
        }
    }
}

impl SmartId {
    /// Generates a new unique trade ID using the current system time and an atomic counter.
    /// The ID is a combination of the lower 16 bits of the current microsecond timestamp and the counter value.
    pub fn new_handle_id() -> HandleId {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_micros() as u16;
        let counter = HANDLE_COUNTER.fetch_add(1, Ordering::SeqCst);
        now.wrapping_add(counter)
    }

    /// Generates a new order ID based on a given handle ID and order kind.
    /// The order ID is a 64-bit value combining the handle ID, order kind, a placeholder for sequence number, and a unique order ID.
    /// The unique order ID is constructed from a timestamp and a counter.
    pub fn new_order_id(trade_id: HandleId, order_kind: HandleOrderKind) -> OrderId {
        let timestamp_part = (SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_micros() % (1 << 16)) as u64;
        let counter_part = ORDER_COUNTER.fetch_add(1, Ordering::SeqCst) as u64;

        let order_kind_part = order_kind as u64;
        let trade_id_part = trade_id as u64;

        let unique_order_id = (timestamp_part << 16) | counter_part; // 32-bit unique order ID

        (trade_id_part << 48) // 16 bits for handle id
            | (order_kind_part << 40) // 8 bits for order kind
            | (0 << 32) // Placeholder 8 bits for sequence number, currently 0
            | unique_order_id // 32 bits for unique order ID
    }

    pub fn new_order_id_with_unique_id(trade_id: HandleId, order_kind: HandleOrderKind, unique_order_id: UniqueOrderId) -> OrderId {
        let order_kind_part = order_kind as u64;
        let trade_id_part = trade_id as u64;

        (trade_id_part << 48) // 16 bits for handle id
            | (order_kind_part << 40) // 8 bits for order kind
            | (0 << 32) // Placeholder 8 bits for sequence number, currently 0
            | unique_order_id as u64 // 32 bits for unique order ID
    }

    /// Decodes a `SmartId` into its constituent parts: handle ID, order kind, unique order ID, and sequence number.
    pub fn decode(self) -> (HandleId, HandleOrderKind, OrderSequenceNumber, UniqueOrderId) {
        let handle_id_mask = ((1 << HANDLE_ID_BITS) - 1) << (ORDER_KIND_BITS + ORDER_SEQUENCE_NUMBER_BITS + UNIQUE_ORDER_ID_BITS);
        let order_kind_mask = ((1 << ORDER_KIND_BITS) - 1) << (ORDER_SEQUENCE_NUMBER_BITS + UNIQUE_ORDER_ID_BITS);
        let sequence_number_mask = ((1 << ORDER_SEQUENCE_NUMBER_BITS) - 1) << UNIQUE_ORDER_ID_BITS;
        let unique_order_id_mask = (1 << UNIQUE_ORDER_ID_BITS) - 1;

        let handle_id = ((self.0 & handle_id_mask) >> (ORDER_KIND_BITS + ORDER_SEQUENCE_NUMBER_BITS + UNIQUE_ORDER_ID_BITS)) as u16;
        let order_kind_bits = ((self.0 & order_kind_mask) >> (ORDER_SEQUENCE_NUMBER_BITS + UNIQUE_ORDER_ID_BITS)) as u8;
        let unique_order_id = (self.0 & unique_order_id_mask) as u32;
        let sequence_number = ((self.0 & sequence_number_mask) >> UNIQUE_ORDER_ID_BITS) as u8;

        (handle_id, HandleOrderKind::from(order_kind_bits), sequence_number, unique_order_id)
    }

    /// Increments the sequence number of a `SmartId`, returning a new `OrderId` or an error if there's an overflow.
    /// This method ensures that only the sequence number part of the `SmartId` is modified.
    pub fn new_incremented_id(&self) -> Result<OrderId, SmartIdError> {
        let (_, _, sequence_number, _) = self.decode();
        let incremented_sequence_number = sequence_number.checked_add(1).ok_or(SmartIdError::SequenceNumberOverflow)?;

        // Clear the current sequence number bits and set the incremented sequence number
        let cleared_id = self.0 & !(0xFF << UNIQUE_ORDER_ID_BITS);
        let new_id = cleared_id | ((incremented_sequence_number as u64) << UNIQUE_ORDER_ID_BITS);

        Ok(new_id)
    }

    pub fn decremented_id(&self) -> Result<OrderId, SmartIdError> {
        let (_, _, sequence_number, _) = self.decode();
        let decremented_sequence_number = sequence_number.checked_sub(1).ok_or(SmartIdError::SequenceNumberOverflow)?;

        // Clear the current sequence number bits and set the incremented sequence number
        let cleared_id = self.0 & !(0xFF << UNIQUE_ORDER_ID_BITS);
        let new_id = cleared_id | ((decremented_sequence_number as u64) << UNIQUE_ORDER_ID_BITS);

        Ok(new_id)
    }

    /// Returns the original ID of a `SmartId` (i.e. sequence number 0).
    /// This is useful for tracking the original order ID of an order.
    pub fn original_id(&self) -> OrderId {
        let (handle_id, order_kind, sequence_number, unique_order_id) = self.decode();
        (handle_id as u64) << (ORDER_KIND_BITS + ORDER_SEQUENCE_NUMBER_BITS + UNIQUE_ORDER_ID_BITS)
            | (order_kind as u64) << (ORDER_SEQUENCE_NUMBER_BITS + UNIQUE_ORDER_ID_BITS)
            | 0u64 << UNIQUE_ORDER_ID_BITS
            | unique_order_id as u64
    }

    /// Returns the handle ID of a `SmartId`.
    pub fn handle_id(self) -> HandleId {
        self.decode().0
    }

    /// Returns the order kind of a `SmartId`.
    pub fn order_kind(self) -> HandleOrderKind {
        self.decode().1
    }

    /// Returns the sequence number of a `SmartId`.
    pub fn sequence_number(self) -> OrderSequenceNumber {
        self.decode().2
    }

    /// Returns the unique order ID of a `SmartId`.
    pub fn unique_order_id(self) -> UniqueOrderId {
        self.decode().3
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_handle_id_unique() {
        let id1 = SmartId::new_handle_id();
        let id2 = SmartId::new_handle_id();

        assert_ne!(id1, id2, "Trade IDs should be unique");

        let mut ids = std::collections::HashSet::new();
        for _ in 0..1000 {
            let id = SmartId::new_handle_id();
            assert!(ids.insert(id), "Duplicate ID generated: {}", id);
        }
    }

    #[test]
    fn test_new_unique_order_id() {
        let handle_id = SmartId::new_handle_id();
        let expected_encoded_type = HandleOrderKind::EntryOrder;
        let local_order_id = SmartId::new_order_id(handle_id, expected_encoded_type);

        eprintln!("handle_id = {:#?}", handle_id);
        eprintln!("local_order_id = {:#?}", local_order_id);
        eprintln!("Id(local_order_id).unique_order_id() = {:#?}", SmartId(local_order_id).unique_order_id());
        eprintln!("order_kind = {:#?}", expected_encoded_type);
        eprintln!("Id(order_id).decode() = {:#?}", SmartId(local_order_id).decode());
        eprintln!("Id(local_order_id).order_kind() = {:#?}", SmartId(local_order_id).order_kind());
        eprintln!("Id(local_order_id).handle_id() = {:#?}", SmartId(local_order_id).handle_id());

        assert_eq!(SmartId(local_order_id).handle_id(), handle_id, "Trade ID should match");
        assert_eq!(SmartId(local_order_id).order_kind(), HandleOrderKind::EntryOrder, "Order type should match");
    }

    #[test]
    fn test_id_decoding() {
        let handle_id = 12345;
        let order_kind = HandleOrderKind::ExitOrder;
        let order_id = SmartId::new_order_id(handle_id, order_kind);

        let (decoded_handle_id, decoded_order_type, sequence_number, decoded_order_id) = SmartId(order_id).decode();
        assert_eq!(decoded_handle_id, handle_id, "Decoded handle ID should match");
        assert_eq!(decoded_order_type, HandleOrderKind::ExitOrder, "Decoded order type should match");
        assert_eq!(decoded_order_id, SmartId(order_id).unique_order_id(), "Decoded order ID should match");
        assert_eq!(sequence_number, 0, "Decoded sequence number should be 0");
    }

    #[test]
    fn test_id_encoding_parts() {
        let handle_id = SmartId::new_handle_id();
        let order_kind = HandleOrderKind::StoplossOrder;
        let order_id1 = SmartId::new_order_id(handle_id, order_kind);
        let order_id2 = SmartId::new_order_id(handle_id, order_kind);
        let order_id3 = SmartId::new_order_id(handle_id, order_kind);

        assert!(SmartId(order_id1).handle_id() > 0, "Trade ID should be positive");
        assert_ne!(SmartId(order_id1).order_kind(), HandleOrderKind::None, "Order type should be positive");
        assert!(SmartId(order_id1).unique_order_id() > 0, "Order ID should be positive");
        assert_eq!(SmartId(order_id1).sequence_number(), 0, "Sequence number should be 0");
        assert_eq!(SmartId(order_id1).handle_id(), SmartId(order_id2).handle_id(), "Trade ID should be the same");

        assert!(SmartId(order_id2).handle_id() > 0, "Trade ID should be positive");
        assert_ne!(SmartId(order_id2).order_kind(), HandleOrderKind::None, "Order type should be positive");
        assert!(SmartId(order_id2).unique_order_id() > 0, "Order ID should be positive");
        assert_eq!(SmartId(order_id2).sequence_number(), 0, "Sequence number should be 0");
        assert_eq!(SmartId(order_id2).handle_id(), SmartId(order_id3).handle_id(), "Trade ID should be the same");

        assert!(SmartId(order_id3).handle_id() > 0, "Trade ID should be positive");
        assert_ne!(SmartId(order_id3).order_kind(), HandleOrderKind::None, "Order type should be positive");
        assert!(SmartId(order_id3).unique_order_id() > 0, "Order ID should be positive");
        assert_eq!(SmartId(order_id3).sequence_number(), 0, "Sequence number should be 0");
        assert_eq!(SmartId(order_id3).handle_id(), SmartId(order_id1).handle_id(), "Trade ID should be the same");
    }

    #[test]
    fn test_sequence_number_extraction() {
        // Create a new order ID with a known handle ID and order type
        let handle_id = SmartId::new_handle_id();
        let order_kind = HandleOrderKind::EntryOrder;
        let original_order_id = SmartId::new_order_id(handle_id, order_kind);

        // Check the initial sequence number (should be 0 for a new order)
        assert_eq!(SmartId(original_order_id).sequence_number(), 0, "Initial sequence number should be 0");

        // Increment the sequence number
        let incremented_order_id = SmartId(original_order_id).new_incremented_id().expect("Sequence number should not overflow");

        // Check the incremented sequence number
        assert_eq!(SmartId(incremented_order_id).sequence_number(), 1, "Sequence number should increment by 1");
        assert_eq!(SmartId(incremented_order_id).unique_order_id(), SmartId(original_order_id).unique_order_id(), "Order ID should not change");
        assert_eq!(SmartId(incremented_order_id).order_kind(), SmartId(original_order_id).order_kind(), "Order type should not change");
        assert_eq!(SmartId(incremented_order_id).handle_id(), SmartId(original_order_id).handle_id(), "Trade ID should not change");
        assert_eq!(SmartId(incremented_order_id).original_id(), original_order_id, "Original order ID should not change");

        // Increment the sequence number again
        let incremented_order_id = SmartId(incremented_order_id).new_incremented_id().expect("Sequence number should not overflow");

        // Check the incremented sequence number
        assert_eq!(SmartId(incremented_order_id).sequence_number(), 2, "Sequence number should increment by 1");
        assert_eq!(SmartId(incremented_order_id).unique_order_id(), SmartId(original_order_id).unique_order_id(), "Order ID should not change");
        assert_eq!(SmartId(incremented_order_id).order_kind(), SmartId(original_order_id).order_kind(), "Order type should not change");
        assert_eq!(SmartId(incremented_order_id).handle_id(), SmartId(original_order_id).handle_id(), "Trade ID should not change");
        assert_eq!(SmartId(incremented_order_id).original_id(), original_order_id, "Original order ID should not change");

        // Decrement the sequence number
        let decremented_order_id = SmartId(incremented_order_id).decremented_id().expect("Sequence number should not overflow");

        // Check the decremented sequence number
        assert_eq!(SmartId(decremented_order_id).sequence_number(), 1, "Sequence number should decrement by 1");
        assert_eq!(SmartId(decremented_order_id).unique_order_id(), SmartId(original_order_id).unique_order_id(), "Order ID should not change");
        assert_eq!(SmartId(decremented_order_id).order_kind(), SmartId(original_order_id).order_kind(), "Order type should not change");
        assert_eq!(SmartId(decremented_order_id).handle_id(), SmartId(original_order_id).handle_id(), "Trade ID should not change");
        assert_eq!(SmartId(decremented_order_id).original_id(), original_order_id, "Original order ID should not change");

        // Decrement the sequence number again
        let decremented_order_id = SmartId(decremented_order_id).decremented_id().expect("Sequence number should not overflow");

        // Check the decremented sequence number
        assert_eq!(SmartId(decremented_order_id).sequence_number(), 0, "Sequence number should decrement by 1");
        assert_eq!(SmartId(decremented_order_id).unique_order_id(), SmartId(original_order_id).unique_order_id(), "Order ID should not change");
        assert_eq!(SmartId(decremented_order_id).order_kind(), SmartId(original_order_id).order_kind(), "Order type should not change");
        assert_eq!(SmartId(decremented_order_id).handle_id(), SmartId(original_order_id).handle_id(), "Trade ID should not change");
        assert_eq!(SmartId(decremented_order_id).original_id(), original_order_id, "Original order ID should not change");

        // Decrement the sequence number again (should fail)
        let decremented_order_id = SmartId(decremented_order_id).decremented_id();
        assert!(decremented_order_id.is_err(), "Sequence number should overflow");
    }
}
