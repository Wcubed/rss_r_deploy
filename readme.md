Script to deploy the [rss_r](https://github.com/Wcubed/rss_r) application to a raspberry pi.

**Be very careful what you specify as directories.**
**If you select the wrong directories, you might delete stuff on the rpi that you didn't want to delete.**

- For uploading to a test directory: `cargo run`
- For uploading to production (only overwrites the `rss_r` executable and `static` directory, leaves configuration intact) `cargo run -- -p`.