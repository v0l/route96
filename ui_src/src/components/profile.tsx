import { hexToBech32 } from "@snort/shared";
import { NostrLink } from "@snort/system";
import { useUserProfile } from "@snort/system-react";
import { useMemo } from "react";

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
  const s = size ?? 40;
  
  const href = adminMode 
    ? `/admin/user/${linkId}`
    : `https://snort.social/${link.encode()}`;
  
  return (
    <a
      className="flex gap-2 items-center"
      href={href}
      target={adminMode ? "_self" : "_blank"}
    >
      <img
        src={
          profile?.picture ||
          `https://nostr.api.v0l.io/api/v1/avatar/cyberpunks/${link.id}`
        }
        alt={profile?.display_name || profile?.name || "User avatar"}
        width={s}
        height={s}
        className="rounded-full object-fit owbject-center"
        onError={(e) => {
          const target = e.target as HTMLImageElement;
          target.src = `https://nostr.api.v0l.io/api/v1/avatar/cyberpunks/${link.id}`;
        }}
      />
      {(showName ?? true) && (
        <div>
          {profile?.display_name ??
            profile?.name ??
            hexToBech32("npub", link.id).slice(0, 12)}
        </div>
      )}
    </a>
  );
}
