import {
  Anchor,
  Button,
  Center,
  Code,
  Loader,
  Stack,
  Text,
  Title,
} from "@mantine/core";
import { Link, useNavigate } from "@tanstack/react-router";
import { useEffect } from "react";
import { useSeriesLookup } from "@/api/queries";
import { seriesLookupRoute } from "@/router";

/// Search params for the `/series/lookup` deep link. `source` and `id` are
/// frozen vocabulary: external tools (Codex's plugin web-links button) bake
/// them into their URL templates, so they stay even though the API speaks
/// `provider`/`externalId`. `title` is optional and only feeds the miss
/// state's feed-search shortcut.
///
/// `id` is `string | number` on purpose: the default TanStack search codec is
/// JSON-based and type-preserving, so `id=4623` arrives as a number, and
/// coercing it to a string here would make `stringifySearch` rewrite the
/// address bar as `id=%224623%22` (JSON-quoted) to keep the string type on a
/// round-trip. The value is stringified at the point of use instead.
export interface LookupSearch {
  source?: string;
  id?: string | number;
  title?: string;
}

export function validateLookupSearch(
  raw: Record<string, unknown>,
): LookupSearch {
  const search: LookupSearch = {};
  if (typeof raw.source === "string" && raw.source.trim())
    search.source = raw.source.trim();
  if (typeof raw.id === "string" && raw.id.trim()) search.id = raw.id.trim();
  else if (typeof raw.id === "number" && Number.isFinite(raw.id))
    search.id = raw.id;
  if (typeof raw.title === "string" && raw.title.trim())
    search.title = raw.title.trim();
  return search;
}

/// Resolver page behind external deep links: looks the `(source, id)` pair up
/// via the API and replace-navigates to the series detail page, so the
/// resolver URL never lingers in history. A miss is the expected outcome for
/// series tsundoku has not discovered (Codex shows its button on owned series
/// too), so it renders as guidance, not an error.
export function SeriesLookupPage() {
  const { source, id, title } = seriesLookupRoute.useSearch();
  const navigate = useNavigate();
  // The API takes the id as a string; the search param keeps its parsed type
  // (see LookupSearch). `!== undefined` rather than truthiness so a numeric
  // id of 0 still counts as present.
  const idString = id !== undefined ? String(id) : undefined;
  const lookup = useSeriesLookup(source, idString);
  const seriesId = lookup.data;

  useEffect(() => {
    if (typeof seriesId === "number") {
      navigate({
        to: "/series/$id",
        params: { id: String(seriesId) },
        replace: true,
      });
    }
  }, [seriesId, navigate]);

  const feedLink = (
    <Anchor component={Link} to="/">
      Go to the feed
    </Anchor>
  );

  if (!source || idString === undefined) {
    return (
      <Center mih="60vh">
        <Stack align="center" gap="sm">
          <Title order={3}>This lookup link is incomplete</Title>
          <Text c="dimmed" ta="center">
            A series lookup needs both a <Code>source</Code> and an{" "}
            <Code>id</Code> query parameter.
          </Text>
          {feedLink}
        </Stack>
      </Center>
    );
  }

  if (lookup.isPending || typeof seriesId === "number") {
    return (
      <Center mih="60vh">
        <Loader />
      </Center>
    );
  }

  if (lookup.isError) {
    return (
      <Center mih="60vh">
        <Stack align="center" gap="sm">
          <Title order={3}>Series lookup failed</Title>
          <Text c="dimmed" ta="center">
            Looking up <Code>{`${source}:${idString}`}</Code> hit an unexpected
            error. Try again in a moment.
          </Text>
          {feedLink}
        </Stack>
      </Center>
    );
  }

  return (
    <Center mih="60vh">
      <Stack align="center" gap="sm">
        <Title order={3}>This series isn't in tsundoku yet</Title>
        <Text c="dimmed" ta="center">
          No discovered series matches <Code>{`${source}:${idString}`}</Code>.
          It shows up here once a release for it is discovered and resolved.
        </Text>
        {title ? (
          // `renderRoot` instead of `component={Link}`: Mantine's polymorphic
          // prop erases the Link generics, so a typed `search` won't compile
          // through it.
          <Button
            renderRoot={(props) => (
              <Link to="/" search={{ q: title }} {...props} />
            )}
          >
            Search the feed for “{title}”
          </Button>
        ) : null}
        {feedLink}
      </Stack>
    </Center>
  );
}
