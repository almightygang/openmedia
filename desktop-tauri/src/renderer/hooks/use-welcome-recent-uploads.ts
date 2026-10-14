import { useEffect, useState } from "react";

import { Episode, StreamProvider } from "@/shared/types";

export function useWelcomeRecentlyUploaded(pageSize: number, provider?: StreamProvider) {
  const [recentUploads, setRecentUploads] = useState<Episode[]>([]);
  const [recentUploadsLoading, setRecentUploadsLoading] = useState(true);

  useEffect(() => {
    let cancelled = false;
    setRecentUploadsLoading(true);
    window.streamProvider
      .getRecentUploads(1, pageSize, provider)
      .then(
        (episodes: Episode[]) => {
          if (cancelled) return;
          setRecentUploads(episodes);
        },
        () => {
          if (!cancelled) setRecentUploads([]);
        }
      )
      .finally(() => {
        if (!cancelled) setRecentUploadsLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [pageSize, provider]);

  return { recentUploads, recentUploadsLoading };
}
