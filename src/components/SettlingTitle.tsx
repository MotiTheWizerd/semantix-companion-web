/** A conversation's name, and the moment it changes.
 *
 * Plain text almost always. While a rename settles (the titler just gave the
 * thread its real name), the old name drifts up and out as the new one rises
 * into its place — the way a departures board flips one line. The caller
 * supplies the old name only for the length of the settle; when it is gone,
 * this is a span with the title in it and nothing else. */
interface SettlingTitleProps {
  title: string;
  /** The name being replaced, for as long as the settle lasts. */
  from?: string | null;
}

export function SettlingTitle({ title, from }: SettlingTitleProps) {
  if (!from) return <span>{title}</span>;
  return (
    <span className="title-settle" key={title}>
      <span className="title-settle__old" aria-hidden="true">
        {from}
      </span>
      <span className="title-settle__new">{title}</span>
    </span>
  );
}
