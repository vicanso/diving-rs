// Image diff view (`?base=…&target=…`): what the target adds, removes and
// grows relative to the base. Data comes from `/api/diff`.

import { memo, useState } from "react";
import { Button, Card, Input, Modal, Tabs } from "antd";
import prettyBytes from "pretty-bytes";
import i18nGet from "./i18n";
import {
  ChangeSet,
  DiffLayer,
  DiffSide,
  FileChange,
  ImageDiff,
  Recommendation,
} from "./types";

const fill = (template: string, values: Record<string, string | number>) =>
  Object.entries(values).reduce(
    (text, [key, value]) => text.replace(`{${key}}`, String(value)),
    template,
  );

const signedBytes = (n: number) =>
  n === 0 ? "0 B" : `${n > 0 ? "+" : "−"}${prettyBytes(Math.abs(n))}`;

const signed = (n: number) => (n > 0 ? `+${n}` : n < 0 ? `−${-n}` : "0");

const percentChange = (from: number, to: number) =>
  from > 0
    ? ` (${to >= from ? "+" : "−"}${((Math.abs(to - from) / from) * 100).toFixed(1)}%)`
    : "";

/** "Compare with…" button: asks for the base image, compares `target`
 *  against it. */
export const CompareButton = ({
  target,
  platform,
  platforms,
  onCompare,
}: {
  target: string;
  /** The platform shown, and the others the image is published for. */
  platform: string;
  platforms: string[];
  onCompare: (base: string, target: string) => void;
}) => {
  const [open, setOpen] = useState(false);
  const [base, setBase] = useState("");
  const submit = () => {
    if (!base.trim()) return;
    setOpen(false);
    onCompare(base.trim(), target);
  };
  // Another platform of this very image: both sides name their platform,
  // so the page's arch setting doesn't apply to either.
  const image = target.split("?arch=")[0];
  const others = platforms.filter((p) => p !== platform);
  const comparePlatform = (other: string) => {
    setOpen(false);
    onCompare(`${image}?arch=${other}`, `${image}?arch=${platform}`);
  };
  return (
    <>
      <Button onClick={() => setOpen(true)}>{i18nGet("compareButton")}</Button>
      <Modal
        open={open}
        title={i18nGet("compareTitle")}
        okText={i18nGet("compareSubmit")}
        okButtonProps={{ disabled: !base.trim() }}
        onOk={submit}
        onCancel={() => setOpen(false)}
        destroyOnHidden
      >
        <p className="compareIntro">
          {fill(i18nGet("compareIntro"), { target })}
        </p>
        <Input
          autoFocus
          value={base}
          placeholder={i18nGet("compareBasePlaceholder")}
          onChange={(e) => setBase(e.target.value)}
          onPressEnter={submit}
        />
        {others.length > 0 && (
          <div className="comparePlatforms">
            <p>
              {fill(i18nGet("comparePlatforms"), {
                platform: platform.toUpperCase(),
              })}
            </p>
            {others.map((p) => (
              <Button key={p} size="small" onClick={() => comparePlatform(p)}>
                {p.toUpperCase()}
              </Button>
            ))}
          </div>
        )}
      </Modal>
    </>
  );
};

/** The two references as the user gave them (the analyses' own names
 *  drop the registry host). */
export interface DiffRefs {
  base: string;
  target: string;
}

type DiffActions = {
  refs: DiffRefs;
  onSwap: () => void;
  onOpen: (image: string) => void;
};

const SideName = ({
  image,
  side,
  label,
}: {
  image: string;
  side: DiffSide;
  label: string;
}) => (
  <div className="diffSide">
    <span className="diffSideLabel">{label}</span>
    <code className="diffSideName">{image}</code>
    {side.staleAsOf && (
      <span className="diffStale">
        {fill(i18nGet("diffStale"), {
          time: new Date(side.staleAsOf).toLocaleString(),
        })}
      </span>
    )}
  </div>
);

const DiffHeader = ({
  diff,
  refs,
  onSwap,
  onOpen,
}: DiffActions & { diff: ImageDiff }) => {
  const { base, target } = diff;
  const rows: [string, string, string, string][] = [
    [
      i18nGet("imageSizeLabel"),
      prettyBytes(base.totalSize),
      prettyBytes(target.totalSize),
      signedBytes(diff.sizeDelta) +
        percentChange(base.totalSize, target.totalSize),
    ],
    [
      i18nGet("diffCompressed"),
      prettyBytes(base.size),
      prettyBytes(target.size),
      signedBytes(target.size - base.size),
    ],
    [
      i18nGet("diffLayers"),
      String(base.layerCount),
      String(target.layerCount),
      signed(target.layerCount - base.layerCount),
    ],
    [
      i18nGet("imageScoreLabel"),
      `${base.efficiencyScore}%`,
      `${target.efficiencyScore}%`,
      signed(target.efficiencyScore - base.efficiencyScore),
    ],
    [
      i18nGet("wastedSizeLabel"),
      prettyBytes(base.wastedSize),
      prettyBytes(target.wastedSize),
      signedBytes(target.wastedSize - base.wastedSize),
    ],
    [
      i18nGet("diffFiles"),
      base.fileCount.toLocaleString(),
      target.fileCount.toLocaleString(),
      signed(target.fileCount - base.fileCount),
    ],
  ];
  return (
    <section className="panel diffHeader">
      <div className="diffHeaderTop">
        <div className="diffSides">
          <SideName image={refs.base} side={base} label={i18nGet("diffBase")} />
          <span className="diffArrow" aria-hidden>
            →
          </span>
          <SideName
            image={refs.target}
            side={target}
            label={i18nGet("diffTarget")}
          />
        </div>
        <div className="diffActions">
          <Button size="small" onClick={onSwap}>
            {i18nGet("diffSwap")}
          </Button>
          <Button size="small" onClick={() => onOpen(refs.base)}>
            {i18nGet("diffOpenBase")}
          </Button>
          <Button size="small" onClick={() => onOpen(refs.target)}>
            {i18nGet("diffOpenTarget")}
          </Button>
        </div>
      </div>
      <div className="diffHero">
        <div className="diffHeroLabel">{i18nGet("diffSizeChange")}</div>
        <div className="diffHeroValue">{signedBytes(diff.sizeDelta)}</div>
        <div className="diffHeroSub">
          {prettyBytes(base.totalSize)} → {prettyBytes(target.totalSize)}
          {percentChange(base.totalSize, target.totalSize)}
        </div>
      </div>
      <table className="diffTable diffMetrics">
        <thead>
          <tr>
            <th />
            <th>{i18nGet("diffBase")}</th>
            <th>{i18nGet("diffTarget")}</th>
            <th>{i18nGet("diffChange")}</th>
          </tr>
        </thead>
        <tbody>
          {rows.map(([label, b, t, change]) => (
            <tr key={label}>
              <th scope="row">{label}</th>
              <td>{b}</td>
              <td>{t}</td>
              <td>{change}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </section>
  );
};

const LayerColumn = ({
  title,
  layers,
}: {
  title: string;
  layers: DiffLayer[];
}) => (
  <div className="diffLayerColumn">
    <h4>
      {title} <span>({layers.length})</span>
    </h4>
    {layers.length === 0 ? (
      <p className="diffEmpty">—</p>
    ) : (
      <ol>
        {layers.map((l) => (
          <li key={`${l.layer}-${l.digest}`}>
            <div className="diffLayerHead">
              <span>{fill(i18nGet("treemapLayer"), { layer: l.layer })}</span>
              <strong>{prettyBytes(l.unpackSize)}</strong>
            </div>
            <code title={l.cmd}>{l.cmd || "—"}</code>
          </li>
        ))}
      </ol>
    )}
  </div>
);

const DiffLayers = ({ diff }: { diff: ImageDiff }) => (
  <Card className="panel" title={i18nGet("diffLayersTitle")}>
    <div className={diff.sharedLayers > 0 ? "diffShared" : "diffShared none"}>
      {diff.sharedLayers > 0
        ? fill(i18nGet("diffShared"), {
            n: diff.sharedLayers,
            size: prettyBytes(diff.sharedSize),
          })
        : i18nGet("diffNoShared")}
    </div>
    <div className="diffLayerColumns">
      <LayerColumn title={i18nGet("diffOnlyBase")} layers={diff.baseLayers} />
      <LayerColumn
        title={i18nGet("diffOnlyTarget")}
        layers={diff.targetLayers}
      />
    </div>
  </Card>
);

/** Diverging bars: growth to the right, shrinkage to the left of a center
 *  axis, length relative to the largest change. */
const DiffDirs = ({ diff }: { diff: ImageDiff }) => {
  if (diff.dirs.length === 0) return null;
  const max = Math.max(
    ...diff.dirs.map((d) => Math.abs(d.targetSize - d.baseSize)),
  );
  return (
    <Card className="panel" title={i18nGet("diffDirsTitle")}>
      <div className="diffLegend">
        <span>
          <i className="diffSwatch down" />
          {i18nGet("diffSmaller")}
        </span>
        <span>
          <i className="diffSwatch up" />
          {i18nGet("diffLarger")}
        </span>
      </div>
      <ul className="diffDirs">
        {diff.dirs.map((d) => {
          const delta = d.targetSize - d.baseSize;
          const width = max > 0 ? (Math.abs(delta) / max) * 50 : 0;
          return (
            <li key={d.path}>
              <code className="diffDirPath">/{d.path}</code>
              <div className="diffBar" aria-hidden>
                <span
                  className={delta >= 0 ? "up" : "down"}
                  style={{ width: `max(${width}%, 2px)` }}
                />
              </div>
              <span className="diffDirDelta">
                <strong>{signedBytes(delta)}</strong>
                <span>
                  {prettyBytes(d.baseSize)} → {prettyBytes(d.targetSize)}
                </span>
              </span>
            </li>
          );
        })}
      </ul>
    </Card>
  );
};

const FileTable = ({
  set,
  kind,
}: {
  set: ChangeSet;
  kind: "added" | "removed" | "changed";
}) => {
  if (set.count === 0) {
    return <p className="diffEmpty">{i18nGet("diffNoFiles")}</p>;
  }
  const size = (f: FileChange) =>
    kind === "added"
      ? prettyBytes(f.targetSize)
      : kind === "removed"
        ? prettyBytes(f.baseSize)
        : `${prettyBytes(f.baseSize)} → ${prettyBytes(f.targetSize)}`;
  return (
    <>
      <table className="diffTable diffFiles">
        <thead>
          <tr>
            <th>{i18nGet("pathLabel")}</th>
            <th>{i18nGet("sizeLabel")}</th>
            {kind === "changed" && <th>{i18nGet("diffChange")}</th>}
            <th>{i18nGet("layerLabel")}</th>
          </tr>
        </thead>
        <tbody>
          {set.files.map((f) => (
            <tr key={f.path}>
              <td>
                <code>{f.path}</code>
              </td>
              <td>{size(f)}</td>
              {kind === "changed" && (
                <td>{signedBytes(f.targetSize - f.baseSize)}</td>
              )}
              <td>{f.layer}</td>
            </tr>
          ))}
        </tbody>
      </table>
      {set.count > set.files.length && (
        <p className="diffNote">
          {fill(i18nGet("diffTopFiles"), {
            n: set.files.length,
            total: set.count.toLocaleString(),
          })}
        </p>
      )}
    </>
  );
};

const DiffFiles = ({ diff }: { diff: ImageDiff }) => {
  const tab = (
    key: "added" | "removed" | "changed",
    label: string,
    set: ChangeSet,
  ) => ({
    key,
    label: `${label} · ${set.count.toLocaleString()} · ${signedBytes(set.bytes)}`,
    children: <FileTable set={set} kind={key} />,
  });
  return (
    <Card className="panel" title={i18nGet("diffFilesTitle")}>
      <Tabs
        items={[
          tab("added", i18nGet("diffAdded"), diff.added),
          tab("removed", i18nGet("diffRemoved"), diff.removed),
          tab("changed", i18nGet("diffResized"), diff.changed),
        ]}
      />
    </Card>
  );
};

const RecList = ({ title, recs }: { title: string; recs: Recommendation[] }) =>
  recs.length === 0 ? null : (
    <div className="diffRecs">
      <h4>
        {title} <span>({recs.length})</span>
      </h4>
      <ul>
        {recs.map((r) => (
          <li key={r.id || r.title}>
            <span className={`diffSeverity ${r.severity}`}>
              {r.severity.toUpperCase()}
            </span>
            <span>{r.title}</span>
            {r.id && <code>{r.id}</code>}
          </li>
        ))}
      </ul>
    </div>
  );

export const DiffView = memo(
  ({ diff, refs, onSwap, onOpen }: DiffActions & { diff: ImageDiff }) => {
    const hasRecs =
      diff.newRecommendations.length > 0 ||
      diff.resolvedRecommendations.length > 0;
    return (
      <>
        <DiffHeader diff={diff} refs={refs} onSwap={onSwap} onOpen={onOpen} />
        <DiffLayers diff={diff} />
        <DiffDirs diff={diff} />
        <DiffFiles diff={diff} />
        {hasRecs && (
          <Card className="panel" title={i18nGet("recommendationsTitle")}>
            <RecList
              title={i18nGet("diffNewRecs")}
              recs={diff.newRecommendations}
            />
            <RecList
              title={i18nGet("diffResolvedRecs")}
              recs={diff.resolvedRecommendations}
            />
          </Card>
        )}
      </>
    );
  },
);
