# Todo

## First
- [ ] Graceful teardown of each thread and recover terminal
- [ ] Fix event [thread dying](https://github.com/sgrimee/webex-tui/issues/1)
- [ ] webex-rust: handle access token expiration/refresh
- [ ] Retrieve messages for a room
- [ ] Retrieve list of all user rooms
- [ ] Display unread messages
- [ ] Make the message list view scrollable
- [ ] Make the rooms list view scrollable

## Next
- [ ] Cache access/refresh token between invocations for faster statup

## Maybe
- [ ] Make callback port configurable
- [ ] Vim bindings

## Done
- [x] Rename the handler and associated messages
- [x] Remove sleep and either select on two channels, or use a single channel (MPSC)
- [x] Use config file for client credentials 

## Will not do
- [ ] webex-rust: fix the device auth flow and drop our teams::auth module (under discussion)