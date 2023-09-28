# Todo

## First
- [ ] Retrieve past messages when viewing a room

## Next
- [ ] Display number of unread messages in list
- [ ] Highlight unread messages in room view
- [ ] Support manual copy of auth URL. Currently hidden by UI.
- [ ] webex-rust: handle access token expiration/refresh
- [ ] webex-rust: retrieve list of all user rooms (needs paging support)
- [ ] Make the message list view scrollable
- [ ] Make the rooms list view scrollable
- [ ] Graceful teardown of each thread
- [ ] Cache access/refresh token between invocations for faster statup

## Maybe
- [ ] Make callback port configurable
- [ ] Modal text editor
- [ ] QR code for auth url (needs available callback endpoint)

## Done
- [x] Room list filter for unread (since app was launched), and recently updated
- [x] Recover terminal on failure
- [x] Fix event [thread dying](https://github.com/sgrimee/webex-tui/issues/1)
- [x] Rename the handler and associated messages
- [x] Remove sleep and either select on two channels, or use a single channel (MPSC)
- [x] Use config file for client credentials 

## Will not do
- [ ] webex-rust: fix the device auth flow and drop our teams::auth module (under discussion)