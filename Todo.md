# Todo

## First
- [ ] Graceful teardown of each thread and recover terminal
- [ ] Fix event thread dying
- [ ] webex-rust: handle access token expiration

## Next
- [ ] Cache access/refresh token between invocations

## Maybe
- [ ] webex-rust: fix the device auth flow and drop our teams::auth module
- [ ] Make callback port configurable

## Done
- [x] Rename the handler and associated messages
- [x] Remove sleep and either select on two channels, or use a single channel (MPSC)
- [x] Use config file for client credentials 
