import { hexToBech32 } from "@snort/shared";
import { NostrLink } from "@snort/system";
import { useUserProfile } from "@snort/system-react";
import { useMemo } from "react";
import { Link } from "react-router-dom";

export default function Profile({
  link,
  size,
  showName,
  adminMode,
}: {
  link: NostrLink;
  size?: number;
  showName?: boolean;
  adminMode?: boolean;
}) {
  const linkId = useMemo(() => link.id, [link.id]);
  const profile = useUserProfile(linkId);
  const s = size ?? 24;
  
  const inner = (
    <>
      <img
        src={
          profile?.picture ||
          `https://nostr-rs-api.v0l.io/avatar/cyberpunks/${link.id}`
        }
        alt={profile?.display_name || profile?.name || "User"}
        width={s}
        height={s}
        className="rounded-sm object-cover"
        onError={(e) => {
          const target = e.target as HTMLImageElement;
          target.src = `https://nostr-rs-api.v0l.io/avatar/cyberpunks/${link.id}`;
        }}
      />
      {(showName ?? true) && (
        <span className="text-xs text-neutral-300">
          {profile?.display_name ??
            profile?.name ??
            hexToBech32("npub", link.id).slice(0, 12)}
        </span>
      )}
    </>
  );

  if (adminMode) {
    return (
      <Link className="flex gap-1.5 items-center hover:text-white" to={`/admin/user/${linkId}`}>
        {inner}
      </Link>
    );
  } else {
    return (
      <a
        className="flex gap-1.5 items-center hover:text-white"
        href={`https://snort.social/${link.encode()}`}
        target="_blank"
      >
        {inner}
      </a>
    );
  }
}
