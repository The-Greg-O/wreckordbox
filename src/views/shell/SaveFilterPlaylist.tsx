import { useEffect, useRef, useState, type FormEvent, type KeyboardEvent, type ReactNode } from "react";
import type { SaveFilterRequest } from "@/ipc/types";
import { suggestedPlaylistName, toSaveRequest } from "@/lib/saveFilterPlaylist";
import { hasPicks, type ShellFilter } from "@/lib/shellTrackFilter";
import { OWN_ACTIVATION } from "@/lib/shortcuts";
import styles from "./SaveFilterPlaylist.module.css";

export interface SaveFilterPlaylistOptions {
  filter: ShellFilter;
  /** The open list's name when it is not All Tracks. */
  within: string | null;
  keyName: (identity: string) => string;
  /** Saves; answers `null` when saved, or why it was not. */
  save: (name: string, request: SaveFilterRequest) => Promise<string | null>;
}

type Said = { kind: "saved"; name: string } | { kind: "refused"; message: string } | null;

/**
 * "Save as intelligent playlist" in the Track Filter bar (wbx-58): a button
 * for the bar's row, and the name form and the outcome under it. The form
 * keeps the filter it was opened on in view; a refusal is shown in the
 * form with the backend's reason, and the form stays open so the DJ can
 * change the filter and save again. Every control owns its activation, so
 * Space and Enter never reach the deck; Escape closes the form.
 */
export function useSaveFilterPlaylist({ filter, within, keyName, save }: SaveFilterPlaylistOptions): { control: ReactNode; panel: ReactNode } {
  const [open, setOpen] = useState(false);
  const [name, setName] = useState("");
  const [busy, setBusy] = useState(false);
  const [said, setSaid] = useState<Said>(null);
  const button = useRef<HTMLButtonElement>(null);
  const field = useRef<HTMLInputElement>(null);
  const empty = !hasPicks(filter) && filter.text.trim() === "";

  // What was said is about the filter it was said of.
  const saidOf = useRef(filter);
  useEffect(() => {
    if (saidOf.current !== filter && said?.kind === "saved") setSaid(null);
    saidOf.current = filter;
  }, [filter, said]);

  useEffect(() => {
    if (open) field.current?.select();
  }, [open]);

  const close = () => {
    setOpen(false);
    button.current?.focus();
  };
  const submit = (event: FormEvent) => {
    event.preventDefault();
    if (busy) return;
    const wanted = name.trim();
    if (wanted === "") {
      setSaid({ kind: "refused", message: "The intelligent playlist needs a name." });
      return;
    }
    setBusy(true);
    void save(wanted, toSaveRequest(filter, within)).then(
      (refusal) => {
        setBusy(false);
        if (refusal === null) {
          setSaid({ kind: "saved", name: wanted });
          close();
        } else {
          setSaid({ kind: "refused", message: refusal });
        }
      },
      (error: unknown) => {
        setBusy(false);
        setSaid({ kind: "refused", message: error instanceof Error ? error.message : String(error) });
      },
    );
  };
  const onKeyDown = (event: KeyboardEvent) => {
    if (event.key === "Escape") {
      event.preventDefault();
      event.stopPropagation();
      close();
    }
  };

  const control = (
    <button
      ref={button}
      type="button"
      className={styles.open}
      aria-expanded={open}
      aria-controls={open ? "save-filter-playlist" : undefined}
      disabled={empty}
      title={empty ? "Pick something in the Track Filter first" : undefined}
      onClick={() => {
        if (open) {
          close();
          return;
        }
        setName(suggestedPlaylistName(filter, keyName));
        setSaid(null);
        setOpen(true);
      }}
    >
      Save as intelligent playlist
    </button>
  );

  const panel = (
    <>
      {open ? (
        <form
          id="save-filter-playlist"
          className={styles.form}
          aria-label="Save the filter as an intelligent playlist"
          onSubmit={submit}
          onKeyDown={onKeyDown}
          {...{ [OWN_ACTIVATION]: "" }}
        >
          <label className={styles.field}>
            Name
            <input ref={field} type="text" aria-label="Intelligent playlist name" value={name}
              onChange={(event) => setName(event.target.value)} />
          </label>
          <button type="submit" className={styles.save} disabled={busy}>{busy ? "Saving…" : "Save"}</button>
          <button type="button" className={styles.cancel} onClick={close}>Cancel</button>
          {said?.kind === "refused" ? <p className={styles.problem} role="alert">Not saved: {said.message}</p> : null}
        </form>
      ) : null}
      {!open && said?.kind === "saved" ? (
        <p className={styles.note} role="status">Saved “{said.name}” as an intelligent playlist.</p>
      ) : null}
    </>
  );

  return { control, panel };
}
