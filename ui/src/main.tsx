// Bookings' own interface (Ferrum docs/plan/apps-ui.md §13) at /bookings/: the team's schedule on its
// home, today's list, and the booking page's settings, with the rest of its records in lists and forms
// in the records frame. And the public booking page guests use, at /bookings/book/{workspace}: a page
// anyone with the address opens (`shell.public`), so it is its own route tree and never loads the
// frame, which keeps it quick on a phone.

import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { StrictMode, useEffect } from "react";
import { createRoot } from "react-dom/client";
import { createBrowserRouter, Link, Outlet, RouterProvider, useRouteError } from "react-router";
import { routeBase, shell, t } from "@ferrum/client";
import { ConfirmHost, EmptyState, ErrorState, Toaster } from "@ferrum/kit";
import "./styles.css";

/** A page of the records frame, loaded when first opened. */
const page = (name: "FormView" | "ListView" | "ReportView" | "AppSettingsPage" | "ScreenPage") => () =>
  import("./records").then((m) => ({ Component: m[name] }));

const queryClient = new QueryClient({
  defaultOptions: { queries: { retry: false, refetchOnWindowFocus: false, staleTime: 30_000 } },
});

function RouteError() {
  return <ErrorState error={useRouteError()} onRetry={() => window.location.reload()} />;
}

function NotFound() {
  return (
    <EmptyState title={t("Page not found")}>
      <Link to="/">{t("Go to the schedule")}</Link>
    </EmptyState>
  );
}

/** Every page tells the shell it started. */
function Root() {
  useEffect(() => shell().ready(), []);
  return <Outlet />;
}

const router = createBrowserRouter(
  [
    {
      path: "/",
      element: <Root />,
      errorElement: <RouteError />,
      children: [
        // For guests: no sign-in, no frame.
        { path: "book/:tenant", lazy: () => import("./guest/Book").then((m) => ({ Component: m.BookPage })) },
        { path: "book/:tenant/manage/:token", lazy: () => import("./guest/Manage").then((m) => ({ Component: m.ManageRoute })) },
        // Anything else under the public address stays public: a guest is never sent to sign in.
        { path: "book/*", element: <EmptyState title={t("Page not found")} icon="calendar" /> },
        {
          // For the team: signed in, in the frame.
          lazy: () => import("./Team").then((m) => ({ Component: m.Team })),
          children: [
            { index: true, lazy: () => import("./Schedule").then((m) => ({ Component: m.Schedule })) },
            { path: "today", lazy: () => import("./Today").then((m) => ({ Component: m.Today })) },
            { path: "page", lazy: () => import("./Page").then((m) => ({ Component: m.BookingPage })) },
            { path: "b/:id", lazy: () => import("./Open").then((m) => ({ Component: m.OpenBooking })) },
            { path: "new", lazy: () => import("./Open").then((m) => ({ Component: m.NewBookingRoute })) },
            { path: "e/:entity", lazy: page("ListView") },
            { path: "e/:entity/:id", lazy: page("FormView") },
            { path: "r/:name", lazy: page("ReportView") },
            { path: "apps/:app/settings", lazy: page("AppSettingsPage") },
            { path: "x/:app/:page", lazy: page("ScreenPage") },
            { path: "*", element: <NotFound /> },
          ],
        },
      ],
    },
  ],
  { basename: routeBase("bookings") },
);

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <QueryClientProvider client={queryClient}>
      <RouterProvider router={router} />
      <Toaster />
      <ConfirmHost />
    </QueryClientProvider>
  </StrictMode>,
);
