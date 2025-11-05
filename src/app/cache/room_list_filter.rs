use enum_iterator::Sequence;

/// Filters used to present a subset of all available rooms.
#[derive(Clone, Debug, PartialEq, Default, Sequence)]
pub(crate) enum RoomsListFilter {
    /// All available rooms
    #[default]
    All,
    /// Only direct messages
    Direct,
    /// Only rooms with recent activity
    Recent,
    /// Only spaces
    Spaces,
    /// Only rooms with unread messages
    Unread,
    /// Only spaces (not 1-1 chats) with no activity for a long time (configurable threshold)
    InactiveSpaces,
}
