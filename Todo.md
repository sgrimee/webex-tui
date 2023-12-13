# Todo

## Next

- [ ] Search for rooms/users
- [ ] Send messages to a new room/user
- [ ] Highlight unread messages in room view
- [ ] webex-rust: handle access token expiration/refresh
- [ ] Cache access/refresh token between invocations for faster statup
- [ ] Cache room/messages data on disk for faster startup
- [ ] Display reactions to messages
- [ ] Send reactions to messages
- [ ] Message attachments
- [ ] Message forwarding
- [ ] Display number of unread messages in list

## Maybe

- [ ] Modal text editor
- [ ] Room sections / favourites

## Done

- [x] Make the logs view scrollable
- [x] webex-rust: retrieve list of all user rooms (limited to 1000)
- [x] Make the message list view scrollable
- [x] Make the rooms list view scrollable
- [x] Graceful teardown of each thread
- [x] Support manual copy of auth URL. Currently hidden by UI.
- [x] Retrieve past messages when viewing a room
- [x] Room list filter for unread (since app was launched), and recently updated
- [x] Recover terminal on failure
- [x] Fix event [thread dying](https://github.com/sgrimee/webex-tui/issues/1)
- [x] Rename the handler and associated messages
- [x] Remove sleep and either select on two channels, or use a single channel (MPSC)
- [x] Use config file for client credentials
