# Third-party content parser dependencies

Ember.Content's own code is covered by the repository's MIT license. Cargo downloads and links
the following libraries; no parser implementation is copied into this directory.

| Library | Direct purpose | Upstream | License |
| --- | --- | --- | --- |
| scraper 0.24 | HTML5 document tree | https://github.com/causal-agent/scraper | ISC |
| html5ever 0.35 (through scraper) | HTML5 tokenization, entities and tree construction | https://github.com/servo/html5ever | MIT OR Apache-2.0 |
| url 2.5 | HTTP/HTTPS parsing and normalization | https://github.com/servo/rust-url | MIT OR Apache-2.0 |

The resolved versions and complete transitive dependency graph are in the repository's
`Cargo.lock`. Their source distributions carry their license files. Selected license texts are
retained in `licenses/` for inclusion in a distributed application's third-party notices.
