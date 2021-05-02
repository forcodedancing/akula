use crate::dbutils;
use async_trait::async_trait;
use bytes::Bytes;
use dbutils::{DupSort, Table};
use ethereum_types::Address;
use futures_core::stream::LocalBoxStream;
use std::{cmp::Ordering, pin::Pin};

pub type ComparatorFunc = Pin<Box<dyn Fn(&[u8], &[u8], &[u8], &[u8]) -> Ordering>>;

#[async_trait(?Send)]
pub trait KV {
    type Tx<'tx>: Transaction<'tx>;

    async fn begin(&self, flags: u8) -> anyhow::Result<Self::Tx<'_>>;
}

#[async_trait(?Send)]
pub trait MutableKV {
    type MutableTx<'tx>: MutableTransaction<'tx>;

    async fn begin_mutable(&self) -> anyhow::Result<Self::MutableTx<'_>>;
}

#[async_trait(?Send)]
pub trait Transaction<'env>: Sized {
    type Cursor<'tx, B: Table>: Cursor<'tx, B>;
    type CursorDupSort<'tx, B: DupSort>: CursorDupSort<'tx, B>;

    /// Cursor - creates cursor object on top of given table. Type of cursor - depends on table configuration.
    /// If table was created with lmdb.DupSort flag, then cursor with interface CursorDupSort created
    /// Otherwise - object of interface Cursor created
    ///
    /// Cursor, also provides a grain of magic - it can use a declarative configuration - and automatically break
    /// long keys into DupSort key/values.
    async fn cursor<'tx, B>(&'tx self) -> anyhow::Result<Self::Cursor<'tx, B>>
    where
        'env: 'tx,
        B: Table;
    async fn cursor_dup_sort<'tx, B>(&'tx self) -> anyhow::Result<Self::CursorDupSort<'tx, B>>
    where
        'env: 'tx,
        B: DupSort;

    async fn get_one<'tx, B>(&'tx self, key: &[u8]) -> anyhow::Result<Option<Bytes<'tx>>>
    where
        'env: 'tx,
        B: Table,
    {
        let mut cursor = self.cursor::<B>().await?;

        Ok(cursor.seek_exact(key).await?.map(|(k, v)| v))
    }
}

#[async_trait(?Send)]
pub trait MutableTransaction<'env>: Transaction<'env> {
    type MutableCursor<'tx, B: Table>: MutableCursor<'tx, B>;
    type MutableCursorDupSort<'tx, B: DupSort>: MutableCursorDupSort<'tx, B>;

    async fn mutable_cursor<'tx, B>(&'tx self) -> anyhow::Result<Self::MutableCursor<'tx, B>>
    where
        'env: 'tx,
        B: Table;
    async fn mutable_cursor_dupsort<'tx, B>(
        &'tx self,
    ) -> anyhow::Result<Self::MutableCursorDupSort<'tx, B>>
    where
        'env: 'tx,
        B: DupSort;

    async fn commit(self) -> anyhow::Result<()>;

    async fn table_size<B: Table>(&self) -> anyhow::Result<u64>;

    /// Allows to create a linear sequence of unique positive integers for each table.
    /// Can be called for a read transaction to retrieve the current sequence value, and the increment must be zero.
    /// Sequence changes become visible outside the current write transaction after it is committed, and discarded on abort.
    /// Starts from 0.
    async fn sequence<B: Table>(&self, amount: usize) -> anyhow::Result<usize>;
}

#[async_trait(?Send)]
pub trait Cursor<'tx, B>
where
    B: Table,
{
    async fn first(&mut self) -> anyhow::Result<Option<(Bytes<'tx>, Bytes<'tx>)>>;
    async fn seek(&mut self, key: &[u8]) -> anyhow::Result<Option<(Bytes<'tx>, Bytes<'tx>)>>;
    async fn seek_exact(&mut self, key: &[u8]) -> anyhow::Result<Option<(Bytes<'tx>, Bytes<'tx>)>>;
    async fn next(&mut self) -> anyhow::Result<Option<(Bytes<'tx>, Bytes<'tx>)>>;
    async fn prev(&mut self) -> anyhow::Result<Option<(Bytes<'tx>, Bytes<'tx>)>>;
    async fn last(&mut self) -> anyhow::Result<Option<(Bytes<'tx>, Bytes<'tx>)>>;
    async fn current(&mut self) -> anyhow::Result<Option<(Bytes<'tx>, Bytes<'tx>)>>;
}

#[async_trait(?Send)]
pub trait MutableCursor<'tx, B>: Cursor<'tx, B>
where
    B: Table,
{
    /// Put based on order
    async fn put(&mut self, key: &[u8], value: &[u8]) -> anyhow::Result<()>;
    /// Append the given key/data pair to the end of the database.
    /// This option allows fast bulk loading when keys are already known to be in the correct order.
    async fn append(&mut self, key: &[u8], value: &[u8]) -> anyhow::Result<()>;
    /// Short version of SeekExact+DeleteCurrent or SeekBothExact+DeleteCurrent
    async fn delete(&mut self, key: &[u8], value: &[u8]) -> anyhow::Result<()>;

    /// Deletes the key/data pair to which the cursor refers.
    /// This does not invalidate the cursor, so operations such as MDB_NEXT
    /// can still be used on it.
    /// Both MDB_NEXT and MDB_GET_CURRENT will return the same record after
    /// this operation.
    async fn delete_current(&mut self) -> anyhow::Result<()>;

    /// Fast way to calculate amount of keys in table. It counts all keys even if prefix was set.
    async fn count(&mut self) -> anyhow::Result<usize>;
}

#[async_trait(?Send)]
pub trait CursorDupSort<'tx, B>: Cursor<'tx, B>
where
    B: DupSort,
{
    async fn seek_both_range(
        &mut self,
        key: &[u8],
        value: &[u8],
    ) -> anyhow::Result<Option<Bytes<'tx>>>;
    /// Position at next data item of current key
    async fn next_dup(&mut self) -> anyhow::Result<Option<(Bytes<'tx>, Bytes<'tx>)>>;
    /// Position at first data item of next key
    async fn next_no_dup(&mut self) -> anyhow::Result<Option<(Bytes<'tx>, Bytes<'tx>)>>;
}

#[async_trait(?Send)]
pub trait MutableCursorDupSort<'tx, B>: MutableCursor<'tx, B> + CursorDupSort<'tx, B>
where
    B: DupSort,
{
    /// Deletes all of the data items for the current key
    async fn delete_current_duplicates(&mut self) -> anyhow::Result<()>;
    /// Same as `Cursor::append`, but for sorted dup data
    async fn append_dup(&mut self, key: &[u8], value: &[u8]) -> anyhow::Result<()>;
}

#[async_trait(?Send)]
pub trait HasStats: Send {
    /// DB size
    async fn disk_size(&self) -> anyhow::Result<u64>;
}

pub struct SubscribeReply;

#[async_trait(?Send)]
pub trait Backend: Send {
    async fn add_local(&self, v: Bytes) -> anyhow::Result<Bytes<'static>>;
    async fn etherbase(&self) -> anyhow::Result<Address>;
    async fn net_version(&self) -> anyhow::Result<u64>;
    async fn subscribe(&self) -> anyhow::Result<LocalBoxStream<'static, SubscribeReply>>;
}