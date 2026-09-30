import { Route, Routes } from "react-router-dom";

import { AppShell } from "@/app/AppShell";
import { CollectScreen } from "@/screens/CollectScreen";
import { LibraryScreen } from "@/screens/LibraryScreen";
import { NotFoundScreen } from "@/screens/NotFoundScreen";
import { ScheduleScreen } from "@/screens/ScheduleScreen";
import { SettingsScreen } from "@/screens/SettingsScreen";
import { TodoScreen } from "@/screens/TodoScreen";

/**
 * Routes of the five main menus. Detail screens added later nest under their
 * menu's path (for example `/library/...`), which keeps that menu marked current.
 */
export function App() {
  return (
    <Routes>
      <Route element={<AppShell />}>
        <Route index element={<ScheduleScreen />} />
        <Route path="library/*" element={<LibraryScreen />} />
        <Route path="todo/*" element={<TodoScreen />} />
        <Route path="collect/*" element={<CollectScreen />} />
        <Route path="settings/*" element={<SettingsScreen />} />
        <Route path="*" element={<NotFoundScreen />} />
      </Route>
    </Routes>
  );
}
