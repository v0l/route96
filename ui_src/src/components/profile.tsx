import { hexToBech32 } from "@snort/shared";
import { NostrLink } from "@snort/system";
import { useUserProfile } from "@snort/system-react";
import { useMemo } from "react";

export default function Profile({
  link,
  size,
  showName,
}: {
  link: NostrLink;
  size?: number;
  showName?: boolean;
}) {
  const linkId = useMemo(() => link.id, [link.id]);
  const profile = useUserProfile(linkId);
  const s = size ?? 40;
  return (
    <a className="flex gap-2 items-center" href={`https://snort.social/${link.encode()}`} target="_blank">
      <img
        src={profile?.picture || '/default-avatar.svg'}
        alt={profile?.display_name || profile?.name || 'User avatar'}
        width={s}
        height={s}
        className="rounded-full object-fit object-center"
        onError={(e) => {
          const target = e.target as HTMLImageElement;
          target.src = '/default-avatar.svg';
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
