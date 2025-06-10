import { hexToBech32 } from "@snort/shared";
import { NostrLink } from "@snort/system";
import { useUserProfile } from "@snort/system-react";

export default function Profile({
  link,
  size,
  showName,
}: {
  link: NostrLink;
  size?: number;
  showName?: boolean;
}) {
  const profile = useUserProfile(link.id);
  const s = size ?? 40;
  return (
    <a className="flex gap-2 items-center" href={`https://snort.social/${link.encode()}`} target="_blank">
      <img
        src={profile?.picture}
        width={s}
        height={s}
        className="rounded-full object-fit object-center"
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
