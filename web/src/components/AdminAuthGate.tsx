import {
  Alert,
  Button,
  Container,
  Group,
  Paper,
  PasswordInput,
  Stack,
  Text,
  Title,
} from "@mantine/core";
import { type ReactNode, useState } from "react";
import { useAdminAuth } from "@/stores/auth";

interface AdminAuthGateProps {
  children: ReactNode;
}

// Wraps any view that calls write endpoints. If no admin token is present in
// the persisted store, render a token-entry form instead of the children. The
// token is stored in localStorage; clearing it logs the operator out.
export function AdminAuthGate({ children }: AdminAuthGateProps) {
  const token = useAdminAuth((s) => s.token);
  if (!token) return <AdminLoginCard />;
  return <>{children}</>;
}

function AdminLoginCard() {
  const setToken = useAdminAuth((s) => s.setToken);
  const [value, setValue] = useState("");

  const submit = () => {
    const trimmed = value.trim();
    if (!trimmed) return;
    setToken(trimmed);
    setValue("");
  };

  return (
    <Container size="sm" py="xl">
      <Paper withBorder radius="md" p="xl">
        <Stack gap="md">
          <Title order={3}>Admin sign-in</Title>
          <Text size="sm" c="dimmed">
            The review queue and other write actions are gated by the
            <Text span fw={600} mx={4}>
              admin_token
            </Text>
            from your tsundoku config. The token is stored in this browser's
            localStorage and sent as a bearer header on writes.
          </Text>
          <PasswordInput
            label="Admin token"
            placeholder="Paste your admin_token"
            value={value}
            onChange={(e) => setValue(e.currentTarget.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") submit();
            }}
            autoFocus
            data-testid="admin-token-input"
          />
          <Group justify="flex-end">
            <Button onClick={submit} disabled={!value.trim()}>
              Save token
            </Button>
          </Group>
          <Alert color="gray" variant="light" title="No token configured?">
            <Text size="xs">
              Set{" "}
              <Text span ff="monospace">
                auth.admin_token
              </Text>{" "}
              in the server config and restart. Until then, write endpoints
              return 503.
            </Text>
          </Alert>
        </Stack>
      </Paper>
    </Container>
  );
}
