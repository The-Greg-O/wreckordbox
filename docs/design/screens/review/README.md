# Review

![Review](review-1280.png)

Where the DJ deals with what agents proposed, and looks back at what changed. Source: `../mock.html#review-1280`.

**Behaviour**
- Left: Facts waiting, grouped by Run. Each Run header offers "Accept ≥ confidence" with the count it would accept, "Accept selected" and "Reject all".
- `Y` accepts and `N` rejects the selected row; `Space` loads its track in the player.
- A confidence below 0.6 is drawn in yellow.
- Right: past Runs from the Ledger. "Show changes" lists a Run's before and after values. "Unwind" reverses it; if values have changed since, the button says how many will be flagged and left alone.
- Suggestions for a track still in Intake also appear on the Intake view; accepting in either place clears both.

**States**
- Nothing waiting.
- A Run still in progress (rows arrive live).
- A Run with held Facts and the reason.
