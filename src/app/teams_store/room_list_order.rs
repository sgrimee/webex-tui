use enum_iterator::Sequence;

/// Sorting options for the rooms list.
#[derive(Clone, Debug, PartialEq, Default, Sequence)]
pub enum RoomsListOrder {
    /// Sort by last activity, most recent first
    #[default]
    ByLastActivity,
    /// Sort by title, ascending
    ByTitle,
    /// No sorting
    Unsorted,
}
