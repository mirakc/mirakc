# actlet

> Yet another tiny actor framework that you've probably seen something similar
> somewhere before...

## Hey, did you search crate.io?

We absolutely agree with your objection.

We searched in [crate.io](https://crates.io/search?q=actor) and found many actor
frameworks.  In fact, some of them meet our requirements.  However,
unfortunately, they seem:

* Not to be maintained actively
* Too complicated for our purposes

Implementing a tiny actor framework based on other convenient crates such as
[tokio] is not a difficult task.  You can see one of such examples in the
following nice blog:

* [Actors with Tokio](https://ryhl.io/blog/actors-with-tokio/)

### Why not use actix?

[actix] is one of widely used actor frameworks.  And mirakc used it before, but
we decided to replace it with our own actor framework for the following reasons:

* `async fn Handler::handle()` is needed for simplifying the implementation
* Too complicated, simpler is enough for mirakc

### Why not use tarpc?

[tarpc] was one of candidates.  It works well with [tokio].  And it supports
`async` handlers, but it always waits the completion of an RPC.  In addition, it
doesn't provide a way to send a message synchronously if a transport channel
has enough space for the message (i.e., it doesn't provide `try_send`).

* [Will tarpc wait for completion of a function that returns nothing?](https://github.com/google/tarpc/issues/335)

### Why not publish?

As we agree with your objection previously.  Nobody wants another actor
framework anymore.  That's enough.

## License

Licensed under either of

* Apache License, Version 2.0
  ([LICENSE-APACHE] or http://www.apache.org/licenses/LICENSE-2.0)
* MIT License
  ([LICENSE-MIT] or http://opensource.org/licenses/MIT)

at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in this project by you, as defined in the Apache-2.0 license,
shall be dual licensed as above, without any additional terms or conditions.

[tokio]: https://tokio.rs/
[actix]: https://github.com/actix/actix
[tarpc]: https://github.com/google/tarpc
[LICENSE-APACHE]: ./LICENSE-APACHE
[LICENSE-MIT]: ./LICENSE-MIT
