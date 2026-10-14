import React, { useEffect, useState } from "react";

import { MacTitlebar } from "./titlebar-mac";
import { ControlsTitlebar } from "./titlebar-controls";

export function Titlebar({
  children,
  className,
}: {
  children?: React.ReactNode;
  className?: string;
}) {
  const [os, setOs] = useState<string | null>(null);

  useEffect(() => {
    window.app
      .os()
      .then(setOs)
      .catch(() => setOs(null));
  }, []);

  // Only macOS gets the traffic-lights layout. Windows, Linux, and the
  // pre-detection state all get the layout with min/max/close buttons —
  // the window is undecorated (tauri.conf `decorations: false`), so the
  // custom bar is the only place those controls exist.
  if (os === "darwin") {
    return <MacTitlebar className={className}>{children}</MacTitlebar>;
  }

  return <ControlsTitlebar className={className}>{children}</ControlsTitlebar>;
}
