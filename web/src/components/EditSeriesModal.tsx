import {
  Button,
  Group,
  Modal,
  NumberInput,
  Select,
  Stack,
  TagsInput,
  Textarea,
  TextInput,
} from "@mantine/core";
import { notifications } from "@mantine/notifications";
import { useState } from "react";
import { useUpdateSeries } from "@/api/mutations";
import type { SeriesDetail } from "@/api/queries";
import { KIND_OPTIONS, STATUS_OPTIONS } from "@/constants/series";
import { useIsMobile } from "@/hooks/useIsMobile";

/// Admin-only editor for a *manual* series' descriptive fields. The caller is
/// responsible for only rendering this when `series.metadataSource ===
/// "manual"` (the backend rejects provider-backed rows with 409). Uses
/// controlled `useState` (the project does not use `@mantine/form`).
export function EditSeriesModal({
  series,
  onClose,
}: {
  series: SeriesDetail;
  onClose: () => void;
}) {
  const update = useUpdateSeries();
  const isMobile = useIsMobile();

  const [title, setTitle] = useState(series.canonicalTitle);
  const [alternateTitles, setAlternateTitles] = useState<string[]>(
    series.alternateTitles,
  );
  const [kind, setKind] = useState(series.kind ?? "");
  const [status, setStatus] = useState(series.status ?? "");
  // NumberInput hands back number | string (string while the field is empty
  // or mid-edit); keep the raw value and coerce on submit.
  const [year, setYear] = useState<number | string>(series.year ?? "");
  const [coverUrl, setCoverUrl] = useState(series.coverUrl ?? "");
  const [description, setDescription] = useState(series.description ?? "");

  const titleTrimmed = title.trim();
  const canSubmit = titleTrimmed.length > 0 && !update.isPending;

  // Include the current value in the option list so a pre-existing custom
  // kind/status (set via the API outside the canonical vocab) still renders.
  const withCurrent = (options: string[], current: string) =>
    current && !options.includes(current) ? [current, ...options] : options;

  const handleSubmit = () => {
    if (!canSubmit) return;
    const parsedYear = typeof year === "number" ? year : Number(year);
    update.mutate(
      {
        id: series.id,
        body: {
          canonicalTitle: titleTrimmed,
          alternateTitles,
          kind: kind.trim() || null,
          status: status.trim() || null,
          year:
            Number.isFinite(parsedYear) && parsedYear > 0 ? parsedYear : null,
          coverUrl: coverUrl.trim() || null,
          description: description.trim() || null,
        },
      },
      {
        onSuccess: () => {
          notifications.show({ color: "green", message: "Series updated" });
          onClose();
        },
        onError: (e) =>
          notifications.show({
            color: "red",
            title: "Update failed",
            message: (e as Error).message,
          }),
      },
    );
  };

  return (
    <Modal
      opened
      onClose={onClose}
      title="Edit series"
      size="lg"
      centered
      fullScreen={isMobile}
    >
      <Stack gap="md">
        <TextInput
          label="Title"
          required
          value={title}
          onChange={(e) => setTitle(e.currentTarget.value)}
          error={titleTrimmed.length === 0 ? "Title is required" : undefined}
          data-autofocus
          data-testid="edit-series-title"
        />
        <TagsInput
          label="Alternate titles"
          description="Press Enter to add. These feed search ranking."
          placeholder="Add a title…"
          value={alternateTitles}
          onChange={setAlternateTitles}
          clearable
          data-testid="edit-series-alternate-titles"
        />
        <Group grow align="flex-start">
          <Select
            label="Kind"
            placeholder="Any"
            data={withCurrent(KIND_OPTIONS, kind)}
            value={kind || null}
            onChange={(v) => setKind(v ?? "")}
            clearable
            searchable
          />
          <Select
            label="Status"
            placeholder="Any"
            data={withCurrent(STATUS_OPTIONS, status)}
            value={status || null}
            onChange={(v) => setStatus(v ?? "")}
            clearable
            searchable
          />
          <NumberInput
            label="Year"
            placeholder="2021"
            value={year}
            onChange={setYear}
            min={0}
            allowDecimal={false}
            hideControls
          />
        </Group>
        <TextInput
          label="Cover URL"
          placeholder="https://…"
          value={coverUrl}
          onChange={(e) => setCoverUrl(e.currentTarget.value)}
        />
        <Textarea
          label="Description"
          rows={5}
          value={description}
          onChange={(e) => setDescription(e.currentTarget.value)}
        />
        <Group justify="flex-end">
          <Button variant="default" onClick={onClose}>
            Cancel
          </Button>
          <Button
            onClick={handleSubmit}
            disabled={!canSubmit}
            loading={update.isPending}
            data-testid="edit-series-submit"
          >
            Save changes
          </Button>
        </Group>
      </Stack>
    </Modal>
  );
}
