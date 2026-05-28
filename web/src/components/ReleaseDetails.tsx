import {
  Anchor,
  Badge,
  Box,
  Collapse,
  Group,
  Paper,
  ScrollArea,
  Stack,
  Text,
  Typography,
} from "@mantine/core";
import { useDisclosure } from "@mantine/hooks";
import ReactMarkdown from "react-markdown";
import rehypeSanitize from "rehype-sanitize";
import remarkGfm from "remark-gfm";
import type { components } from "@/types/api.generated";

type ExtractedLinksDto = components["schemas"]["ExtractedLinksDto"];

/// Provider links the source scraped from a release's description. Shared by
/// the review and kept cards so a hand-pasted MangaUpdates / AniList / MAL /
/// MangaDex link is verifiable without leaving the page.
export function ExtractedLinks({
  links,
}: {
  links: ExtractedLinksDto | null | undefined;
}) {
  if (!links) {
    return null;
  }
  const entries: { provider: string; label: string; href: string }[] = [
    ...(links.mangaupdates
      ? [
          {
            provider: "mangaupdates",
            label: "MangaUpdates",
            href: links.mangaupdates,
          },
        ]
      : []),
    ...(links.anilist
      ? [{ provider: "anilist", label: "AniList", href: links.anilist }]
      : []),
    ...(links.mal ? [{ provider: "mal", label: "MAL", href: links.mal }] : []),
    ...(links.mangadex
      ? [
          {
            provider: "mangadex",
            label: "MangaDex",
            href: links.mangadex,
          },
        ]
      : []),
  ];
  if (entries.length === 0) {
    return null;
  }
  return (
    <Group gap={6} wrap="wrap" data-testid="extracted-links">
      <Text size="xs" c="dimmed" tt="uppercase" fw={500}>
        links
      </Text>
      {entries.map((e) => (
        <Anchor
          key={e.provider}
          href={e.href}
          target="_blank"
          rel="noreferrer noopener"
          size="xs"
        >
          <Badge
            size="sm"
            variant="light"
            color="teal"
            style={{ cursor: "pointer" }}
          >
            {e.label}
          </Badge>
        </Anchor>
      ))}
    </Group>
  );
}

/// Collapsible markdown rendering of a release's post description. Nyaa
/// uploaders publish bodies in markdown (the `markdown-text` class on the
/// post page); we render with sanitization and GFM tables so the card matches
/// what the operator would see on Nyaa.
export function ReleaseDescription({
  body,
}: {
  body: string | null | undefined;
}) {
  const [opened, { toggle }] = useDisclosure(false);
  const trimmed = body?.trim();
  if (!trimmed) {
    return null;
  }
  return (
    <Box data-testid="description-block">
      <Group gap={6} wrap="wrap" mb={opened ? 4 : 0}>
        <Text size="xs" c="dimmed" tt="uppercase" fw={500}>
          description
        </Text>
        <Anchor
          component="button"
          type="button"
          size="xs"
          onClick={toggle}
          aria-expanded={opened}
        >
          {opened ? "hide" : "show"}
        </Anchor>
      </Group>
      <Collapse in={opened}>
        <Paper
          withBorder
          radius="sm"
          p="sm"
          bg="var(--mantine-color-default-hover)"
        >
          <Typography fz="sm">
            <ReactMarkdown
              remarkPlugins={[remarkGfm]}
              rehypePlugins={[rehypeSanitize]}
              components={{
                a: ({ href, children }) => (
                  <a href={href} target="_blank" rel="noreferrer noopener">
                    {children}
                  </a>
                ),
              }}
            >
              {trimmed}
            </ReactMarkdown>
          </Typography>
        </Paper>
      </Collapse>
    </Box>
  );
}

/// Collapsible list of the files inside the torrent/archive, as scraped from
/// the source's post page (`ReleaseDto.files`). Lets the operator confirm what
/// a release actually contains (e.g. "Vol. 01-20") and compare it against a
/// candidate series' volume/chapter counts. Renders nothing when the source
/// exposed no file list. Long chapter-level torrents can run to hundreds of
/// entries, so the list scrolls rather than stretching the card.
export function ReleaseFiles({
  files,
}: {
  files: string[] | null | undefined;
}) {
  const [opened, { toggle }] = useDisclosure(false);
  if (!files || files.length === 0) {
    return null;
  }
  return (
    <Box data-testid="files-block">
      <Group gap={6} wrap="wrap" mb={opened ? 4 : 0}>
        <Text size="xs" c="dimmed" tt="uppercase" fw={500}>
          files ({files.length})
        </Text>
        <Anchor
          component="button"
          type="button"
          size="xs"
          onClick={toggle}
          aria-expanded={opened}
        >
          {opened ? "hide" : "show"}
        </Anchor>
      </Group>
      <Collapse in={opened}>
        <Paper
          withBorder
          radius="sm"
          p="sm"
          bg="var(--mantine-color-default-hover)"
        >
          <ScrollArea.Autosize mah={220}>
            <Stack gap={2}>
              {files.map((f, i) => (
                <Text
                  // File names within one torrent can repeat across folders,
                  // so the leaf name alone isn't a stable key; pair it with
                  // the index. The list is read-only and never reordered.
                  // biome-ignore lint/suspicious/noArrayIndexKey: see above
                  key={`${f}-${i}`}
                  size="xs"
                  ff="monospace"
                  style={{ wordBreak: "break-all" }}
                >
                  {f}
                </Text>
              ))}
            </Stack>
          </ScrollArea.Autosize>
        </Paper>
      </Collapse>
    </Box>
  );
}
