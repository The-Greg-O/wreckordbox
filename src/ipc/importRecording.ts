/** Recording failure is separate from the already accepted import result. */
export async function importRecordingWarning<T extends { recordingWarning?: string }>(report: T): Promise<T> {
  if (report.recordingWarning) {
    const { message } = await import("@tauri-apps/plugin-dialog");
    await message(report.recordingWarning, { title: "Import recording", kind: "warning" });
  }
  return report;
}
