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
  const s = size ?? 40;
  
  const inner = <>
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
  </>;

  if(adminMode) {
    return <Link className="flex gap-2 items-center" to={`/admin/user/${linkId}`}>{inner}</Link>
  } else {
    return (
      <a
        className="flex gap-2 items-center"
        href={`https://snort.social/${link.encode()}`}
        target={"_blank"}
      >{inner}</a>
    );
  }
  
}
