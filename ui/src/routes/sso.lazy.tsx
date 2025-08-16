import { createLazyFileRoute } from "@tanstack/react-router";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { ExclamationTriangleIcon } from "@radix-ui/react-icons";
import { useState, useEffect } from "react";
import { useAuth } from "@/contexts/AuthContext";

export const Route = createLazyFileRoute("/sso")({
  component: SSOAuth,
});

const domain = window.location.origin;
const ssoUiUrl = import.meta.env.VITE_SSO_UI_URL;
const baseUrl = "/web/sso";
const loginUrl = `${ssoUiUrl}/login?service=${domain}${baseUrl}`;

function SSOAuth() {
  const { handlers } = useAuth();
  const [error, setError] = useState({ message: "", error_type: "" });

  useEffect(() => {
    const params = new URLSearchParams(window.location.search);
    const ticket = params.get("ticket");

    if (ticket) {
      handlers.loginWithSSO(ticket).catch((err) => {
        if (typeof err === "string") {
          setError({ message: err, error_type: "generic" });
        } else if (err?.error) {
          setError({ message: err.error, error_type: "backend" });
        } else {
          setError({ message: "SSO Login failed", error_type: "unknown" });
        }
      });
    }
  }, []);

  return (
    <div className="flex flex-col w-full h-full min-h-screen justify-center items-center space-y-8">
      {error.message && (
        <Alert
          variant="default"
          className="max-w-lg w-full border-red-400 text-red-400"
        >
          <ExclamationTriangleIcon className="h-5 w-5 mt-0.5 !text-red-400" />
          <AlertTitle className="text-lg font-semibold">
            Login Failed
          </AlertTitle>
          <AlertDescription>{error.message}</AlertDescription>
        </Alert>
      )}

      <Card className="max-w-lg w-full bg-slate-900 border-slate-600 p-2">
        <CardHeader>
          <CardTitle className="text-center text-3xl">
            Login with SSO UI
          </CardTitle>
        </CardHeader>
        <CardContent className="gap-4 flex flex-col items-center justify-center space-y-2">
          <Button
            size="lg"
            className="text-foreground w-2/3"
            onClick={() => window.location.replace(loginUrl)}
          >
            SSO Login
          </Button>
        </CardContent>
      </Card>
    </div>
  );
}
