# Live test suites

pgorm's live tests run against a real PostgreSQL server: the Rust crates'
suites (`cargo nextest`) and pgorm-napi's, under Node.js and Deno. The server
is named by `DATABASE_URL` and is shared, with other checkouts, with other runs
of the same checkout and with the other suites. This chapter specifies what a
run may do on that server so that runs cannot collide. pgorm-python's suite
starts a disposable server of its own, and the security harnesses run in their
own containers, so neither is bound by it.

## Run isolation

> [spec:pgorm:req:testing.isolation+1]
> Any number of runs MAY share the server `DATABASE_URL` names at once: two
> checkouts, two runs of one checkout, the Rust crates' suites and
> pgorm-napi's. A run MUST NOT touch another's databases, roles or connections.
> Each database or role a live test creates is named for the process that
> creates it, `{label}_{seq}__{owner}_{pid}_{stamp}`. The label is the test's
> own, cut so the whole name fits PostgreSQL's 63 bytes. The sequence counts
> the names that process has minted, so two labels cut to one prefix still
> differ. The owner is a hash of the host name and the user, the scope in
> which a process id means anything. The pid is the creating process. The
> stamp is the Unix second at which it minted its first name, which tells the
> creator from a later process given the same pid. The Rust crates mint these
> names through one unpublished crate, `pgorm-test-support`, a dev-dependency
> of pgorm and pgorm-migration that reaches the server through pgorm-driver
> rather than pgorm; pgorm-napi's support module mints the same names.
>
> A run MUST NOT drop a database or role it did not create, so nothing drops a
> prior copy before creating its own. The one exception is a leftover whose
> creator is provably gone: its name records this host and user, no process
> holds its pid (or the one that does started after the stamp), and no session
> is connected to it. Such a database is dropped without `FORCE`, so a session
> that arrives in between makes the drop fail rather than be evicted. Whatever
> the host cannot answer counts as the creator still running. A test that
> terminates a backend MUST name it by pid within its own database. A test
> that runs in the shared `postgres` database, as pgorm-pool's do, MUST keep
> what it creates there to its own session: a temporary table, a session
> setting.
