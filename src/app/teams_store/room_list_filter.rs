use enum_iterator::Sequence;

/// Filters used to present a subset of all available rooms.
#[derive(Clone, Debug, PartialEq, Default, Sequence)]
pub enum RoomsListFilter {
    /// All available rooms
    All,
    /// Only direct messages
    Direct,
    /// Only rooms with recent activity
    #[default]
    Recent,
    /// Only spaces
    Spaces,
    /// Only rooms with unread messages
    Unread,
}
