import {
  ActionIcon,
  Anchor,
  Badge,
  Container,
  Group,
  AppShell as MantineAppShell,
  type MantineColor,
  Text,
  Title,
  useComputedColorScheme,
  useMantineColorScheme,
} from "@mantine/core";
import { Link } from "@tanstack/react-router";
import type { ReactNode } from "react";
import { useAppInfo, useStats } from "@/api/queries";

function ColorSchemeToggle() {
  const { setColorScheme } = useMantineColorScheme();
  const computed = useComputedColorScheme("light", {
    getInitialValueInEffect: true,
  });
  const isDark = computed === "dark";
  return (
    <ActionIcon
      variant="default"
      size="md"
      radius="sm"
      aria-label={isDark ? "Switch to light mode" : "Switch to dark mode"}
      onClick={() => setColorScheme(isDark ? "light" : "dark")}
    >
      {isDark ? (
        <svg
          xmlns="http://www.w3.org/2000/svg"
          width="16"
          height="16"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          strokeWidth="2"
          strokeLinecap="round"
          strokeLinejoin="round"
          aria-hidden="true"
        >
          <circle cx="12" cy="12" r="4" />
          <path d="M12 2v2" />
          <path d="M12 20v2" />
          <path d="m4.93 4.93 1.41 1.41" />
          <path d="m17.66 17.66 1.41 1.41" />
          <path d="M2 12h2" />
          <path d="M20 12h2" />
          <path d="m6.34 17.66-1.41 1.41" />
          <path d="m19.07 4.93-1.41 1.41" />
        </svg>
      ) : (
        <svg
          xmlns="http://www.w3.org/2000/svg"
          width="16"
          height="16"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          strokeWidth="2"
          strokeLinecap="round"
          strokeLinejoin="round"
          aria-hidden="true"
        >
          <path d="M12 3a6 6 0 0 0 9 9 9 9 0 1 1-9-9Z" />
        </svg>
      )}
    </ActionIcon>
  );
}

export function AppShell({ children }: { children: ReactNode }) {
  const stats = useStats();
  const appInfo = useAppInfo();
  const reviewCount =
    (stats.data?.releases.unresolved ?? 0) +
    (stats.data?.releases.ambiguous ?? 0) +
    (stats.data?.releases.reviewPending ?? 0);

  return (
    <MantineAppShell header={{ height: 56 }} padding={0}>
      <MantineAppShell.Header>
        <Container size="xl" h="100%">
          <Group h="100%" justify="space-between" align="center" wrap="nowrap">
            <Anchor component={Link} to="/" underline="never" c="inherit">
              <Group gap="xs" align="baseline">
                <Title order={3}>tsundoku</Title>
                <Text size="xs" c="dimmed">
                  discovery
                </Text>
                {appInfo.data?.version && (
                  <Text size="xs" c="dimmed" fw={400}>
                    v{appInfo.data.version}
                  </Text>
                )}
              </Group>
            </Anchor>
            <Group gap="md" align="center" wrap="nowrap">
              {typeof stats.data?.series === "number" && (
                <Text size="sm" c="dimmed" component="span">
                  {stats.data.series} series
                </Text>
              )}
              <Badge
                component={Link}
                to="/admin/review"
                color={(reviewCount > 0 ? "orange" : "gray") as MantineColor}
                variant={reviewCount > 0 ? "light" : "default"}
                radius="sm"
                style={{ cursor: "pointer", textDecoration: "none" }}
                aria-label={`Review queue (${reviewCount} pending)`}
              >
                {reviewCount > 0 ? `${reviewCount} to review` : "review"}
              </Badge>
              <Badge
                component={Link}
                to="/admin/wishlist"
                color="yellow"
                variant="default"
                radius="sm"
                style={{ cursor: "pointer", textDecoration: "none" }}
                aria-label="Wishlist"
                data-testid="appbar-wishlist"
              >
                ★ wishlist
              </Badge>
              <Badge
                component={Link}
                to="/admin"
                color="grape"
                variant="default"
                radius="sm"
                style={{ cursor: "pointer", textDecoration: "none" }}
                aria-label="Admin"
              >
                admin
              </Badge>
              {stats.data?.activeProvider && (
                <Badge variant="default" radius="sm">
                  {stats.data.activeProvider}
                </Badge>
              )}
              <ColorSchemeToggle />
            </Group>
          </Group>
        </Container>
      </MantineAppShell.Header>
      <MantineAppShell.Main>{children}</MantineAppShell.Main>
    </MantineAppShell>
  );
}
