# The Library service stays offline; agents do the looking up

Finding track information on the web is done by agents with their own tools, not by wreckordbox. Wreckordbox never reaches the network: it supplies a precise description of the track to search for, checks a candidate against the track before a Fact is accepted, and records the Fact with its source address and date. We chose this because site-specific connectors need upkeep as sources change while agents' search improves without work from us, because the process that owns the Library is safer with no network access, and because the hard part of a lookup is matching the right recording, which wreckordbox is placed to check and a fetcher is not.

## Consequences

- Supporting another source (Beatport first, others later) is an agent tool, not a change to wreckordbox.
- Nothing in wreckordbox can look a track up when no agent is present.
