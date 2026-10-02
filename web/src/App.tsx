import { Route, Routes } from "react-router-dom";

import { AppShell } from "@/app/AppShell";
import { CollectScreen } from "@/screens/CollectScreen";
import { LibraryScreen } from "@/screens/LibraryScreen";
import { WorkDetailScreen } from "@/screens/library/WorkDetailScreen";
import { NotFoundScreen } from "@/screens/NotFoundScreen";
import { ScheduleScreen } from "@/screens/ScheduleScreen";
import { SettingsScreen } from "@/screens/SettingsScreen";
import { TodoScreen } from "@/screens/TodoScreen";
import { JobDetailScreen } from "@/screens/todo/JobDetailScreen";

/**
 * Routes of the five main menus. Detail screens nest under their menu's path
 * (`/library/<work>`), which keeps that menu marked current.
 */
export function App() {
  return (
    <Routes>
      <Route element={<AppShell />}>
        <Route index element={<ScheduleScreen />} />
        <Route path="library" element={<LibraryScreen />} />
        <Route path="library/:workId" element={<WorkDetailScreen />} />
        <Route path="library/*" element={<NotFoundScreen />} />
        <Route path="todo" element={<TodoScreen />} />
        <Route path="todo/job/:jobId" element={<JobDetailScreen />} />
        <Route path="todo/*" element={<NotFoundScreen />} />
        <Route path="collect/*" element={<CollectScreen />} />
        <Route path="settings/*" element={<SettingsScreen />} />
        <Route path="*" element={<NotFoundScreen />} />
      </Route>
    </Routes>
  );
}
