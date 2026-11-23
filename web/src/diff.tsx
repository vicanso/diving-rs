// Image diff view (`?base=…&target=…`): what the target adds, removes and
// grows relative to the base. Data comes from `/api/diff`.

import { memo, useMemo, useState } from "react";
import { Button, Card, Checkbox, Input, Modal, Select, Tabs, Tag } from "antd";
import prettyBytes from "pretty-bytes";
import i18nGet from "./i18n";
import {
  BaseLayerSkipReason,
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
  baseMode: string;
  onChangeBaseMode: (mode: string) => void;
  onSwap: () => void;
  onOpen: (image: string) => void;
};

const skipReasonText = (reason: BaseLayerSkipReason) => {
  switch (reason) {
    case "crossArchCreatedGap":
      return i18nGet("diffSkipCreatedGap");
    case "crossArchFirstLayer":
      return i18nGet("diffSkipFirstLayer");
    case "forcedCreatedGap":
      return i18nGet("diffSkipForcedGap");
    case "forcedFirstLayer":
      return i18nGet("diffSkipForcedFirst");
    case "manualCount":
      return i18nGet("diffSkipManual");
    default:
      return "";
  }
};

const isExpectedBinaryOnly = (f: FileChange) =>
  Boolean(f.isBinary) &&
  f.baseSize > 0 &&
  f.targetSize > 0 &&
  (f.baseMode || "") === (f.targetMode || "") &&
  (f.baseLink || "") === (f.targetLink || "");

const ParityBanner = ({ diff }: { diff: ImageDiff }) => {
  const { parity } = diff;
  let headline: string;
  if (parity.status === "consistent") {
    headline = i18nGet("diffParityConsistent");
  } else if (parity.status === "binaryDiffsOnly") {
    headline = fill(i18nGet("diffParityBinaryOnly"), {
      n: parity.expectedBinaryChanges,
    });
  } else {
    headline = fill(i18nGet("diffParityDrift"), {
      files: parity.unexpectedFileChanges,
      config: parity.configChanges,
      runtime: parity.runtimeChanges,
      binaries: parity.expectedBinaryChanges,
    });
  }
  const hasContentStats =
    diff.contentVerifiedFiles > 0 || diff.contentSkippedFiles > 0;
  return (
    <div className={`diffParityBanner ${parity.status}`}>
      <div>{headline}</div>
      {hasContentStats && (
        <div className="diffParityMeta">
          {diff.contentVerified
            ? fill(i18nGet("diffContentVerified"), {
                verified: diff.contentVerifiedFiles,
                skipped: diff.contentSkippedFiles,
              })
            : fill(i18nGet("diffContentUnverified"), {
                skipped: diff.contentSkippedFiles,
              })}
        </div>
      )}
    </div>
  );
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
  baseMode,
  onChangeBaseMode,
  onSwap,
  onOpen,
}: DiffActions & { diff: ImageDiff }) => {
  const { base, target } = diff;
  const baseUpper = diff.baseLayers.reduce((sum, l) => sum + l.unpackSize, 0);
  const targetUpper = diff.targetLayers.reduce(
    (sum, l) => sum + l.unpackSize,
    0,
  );
  const rows: [string, string, string, string][] = [
    [
      i18nGet("imageSizeLabel"),
      prettyBytes(base.totalSize),
      prettyBytes(target.totalSize),
      signedBytes(diff.sizeDelta) +
        percentChange(base.totalSize, target.totalSize),
    ],
    ...(diff.baseLayersIgnored
      ? [
          [
            i18nGet("diffUpperSizeChange"),
            prettyBytes(baseUpper),
            prettyBytes(targetUpper),
            signedBytes(diff.upperSizeDelta) +
              percentChange(baseUpper, targetUpper),
          ] as [string, string, string, string],
        ]
      : []),
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
  const modeOptions = [
    { value: "auto", label: i18nGet("diffModeAuto") },
    { value: "forceAuto", label: i18nGet("diffModeForce") },
    { value: "off", label: i18nGet("diffModeOff") },
    ...[1, 2, 3, 4, 5].map((n) => ({
      value: `skip:${n}`,
      label: fill(i18nGet("diffModeSkipN"), { n }),
    })),
  ];
  const heroDelta = diff.baseLayersIgnored
    ? diff.upperSizeDelta
    : diff.sizeDelta;
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
          <Select
            className="diffModeSelect"
            size="small"
            value={baseMode || "auto"}
            options={modeOptions}
            aria-label={i18nGet("diffModeLabel")}
            onChange={onChangeBaseMode}
          />
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
        <div className="diffHeroLabel">
          {diff.baseLayersIgnored
            ? i18nGet("diffUpperSizeChange")
            : i18nGet("diffSizeChange")}
        </div>
        <div className="diffHeroValue">{signedBytes(heroDelta)}</div>
        <div className="diffHeroSub">
          {diff.baseLayersIgnored
            ? `${prettyBytes(baseUpper)} → ${prettyBytes(targetUpper)}${percentChange(baseUpper, targetUpper)} · ${fill(
                i18nGet("diffFullSizeNote"),
                {
                  delta: signedBytes(diff.sizeDelta),
                  base: prettyBytes(base.totalSize),
                  target: prettyBytes(target.totalSize),
                },
              )}`
            : `${prettyBytes(base.totalSize)} → ${prettyBytes(target.totalSize)}${percentChange(base.totalSize, target.totalSize)}`}
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
      <ParityBanner diff={diff} />
    </section>
  );
};

const DiffConfigRuntime = ({ diff }: { diff: ImageDiff }) => {
  const { configDiff: cd, runtimeDiff: rd } = diff;
  const rows = useMemo(() => {
    const out: [string, string, string][] = [];
    const show = (s: string) => s || "—";
    const cmd = (v: string[]) => (v.length ? v.join(" ") : "—");
    if (cd.baseOs)
      out.push(["Base OS", show(cd.baseOs.base), show(cd.baseOs.target)]);
    if (cd.user) out.push(["USER", show(cd.user.base), show(cd.user.target)]);
    if (cd.workingDir)
      out.push([
        "WORKDIR",
        show(cd.workingDir.base),
        show(cd.workingDir.target),
      ]);
    if (cd.entrypoint)
      out.push([
        "ENTRYPOINT",
        cmd(cd.entrypoint.base),
        cmd(cd.entrypoint.target),
      ]);
    if (cd.cmd) out.push(["CMD", cmd(cd.cmd.base), cmd(cd.cmd.target)]);
    if (cd.hasHealthcheck)
      out.push([
        "HEALTHCHECK",
        String(cd.hasHealthcheck.base),
        String(cd.hasHealthcheck.target),
      ]);
    if (cd.addedPorts?.length || cd.removedPorts?.length) {
      out.push([
        "EXPOSE",
        cd.removedPorts?.join(", ") || "—",
        cd.addedPorts?.join(", ") || "—",
      ]);
    }
    for (const env of cd.envChanges || []) {
      out.push([`ENV ${env.key}`, env.base ?? "—", env.target ?? "—"]);
    }
    for (const label of cd.labelChanges || []) {
      out.push([`LABEL ${label.key}`, label.base ?? "—", label.target ?? "—"]);
    }
    if (rd.entrypoint)
      out.push([
        "Runtime entrypoint",
        show(rd.entrypoint.base),
        show(rd.entrypoint.target),
      ]);
    if (rd.libc)
      out.push(["Runtime libc", show(rd.libc.base), show(rd.libc.target)]);
    if (rd.requiredGlibc)
      out.push([
        "Min glibc",
        show(rd.requiredGlibc.base),
        show(rd.requiredGlibc.target),
      ]);
    if (rd.osGlibc)
      out.push(["OS glibc", show(rd.osGlibc.base), show(rd.osGlibc.target)]);
    if (rd.issue)
      out.push(["Runtime issue", show(rd.issue.base), show(rd.issue.target)]);
    if (rd.addedUnstripped?.length || rd.removedUnstripped?.length) {
      out.push([
        "Unstripped binaries",
        rd.removedUnstripped?.length
          ? `${rd.baseUnstrippedCount} (−${rd.removedUnstripped.join(", ")})`
          : String(rd.baseUnstrippedCount),
        rd.addedUnstripped?.length
          ? `${rd.targetUnstrippedCount} (+${rd.addedUnstripped.join(", ")})`
          : String(rd.targetUnstrippedCount),
      ]);
    }
    return out;
  }, [cd, rd]);

  return (
    <Card className="panel" title={i18nGet("diffConfigRuntimeTitle")}>
      {cd.same && rd.same ? (
        <div className="diffParityBanner consistent">
          {i18nGet("diffConfigRuntimeOk")}
        </div>
      ) : (
        <table className="diffTable diffFiles">
          <thead>
            <tr>
              <th>{i18nGet("diffItemLabel")}</th>
              <th>{i18nGet("diffBase")}</th>
              <th>{i18nGet("diffTarget")}</th>
            </tr>
          </thead>
          <tbody>
            {rows.map(([item, b, t]) => (
              <tr key={item}>
                <td>
                  <code>{item}</code>
                </td>
                <td>
                  <code>{b}</code>
                </td>
                <td>
                  <code>{t}</code>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </Card>
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
    <div
      className={
        diff.baseLayersIgnored || diff.sharedLayers > 0
          ? "diffShared"
          : "diffShared none"
      }
    >
      {diff.baseLayersIgnored
        ? fill(i18nGet("diffSkipped"), {
            base: diff.baseSkippedLayers,
            baseSize: prettyBytes(diff.baseSkippedSize),
            target: diff.targetSkippedLayers,
            targetSize: prettyBytes(diff.targetSkippedSize),
            reason: skipReasonText(diff.baseSkipReason),
          })
        : diff.sharedLayers > 0
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

const FileBadges = ({ f }: { f: FileChange }) => {
  const badges: { key: string; label: string; color?: string }[] = [];
  if (f.isBinary) {
    badges.push({ key: "bin", label: i18nGet("diffBadgeBinary") });
  }
  if (f.contentChanged) {
    badges.push({
      key: "content",
      label: i18nGet("diffBadgeContent"),
      color: "orange",
    });
  }
  if ((f.baseMode || "") !== (f.targetMode || "")) {
    badges.push({
      key: "mode",
      label: `${i18nGet("diffBadgeMode")}: ${f.baseMode || "-"}→${f.targetMode || "-"}`,
      color: "gold",
    });
  }
  if ((f.baseLink || "") !== (f.targetLink || "")) {
    badges.push({
      key: "link",
      label: i18nGet("diffBadgeLink"),
      color: "purple",
    });
  }
  if (badges.length === 0) return null;
  return (
    <div className="diffFileBadges">
      {badges.map((b) => (
        <Tag key={b.key} color={b.color}>
          {b.label}
        </Tag>
      ))}
    </div>
  );
};

const FileTable = ({
  set,
  kind,
  unexpectedOnly,
  onToggleUnexpectedOnly,
  hasExpectedBinaries,
}: {
  set: ChangeSet;
  kind: "added" | "removed" | "changed";
  unexpectedOnly?: boolean;
  onToggleUnexpectedOnly?: (next: boolean) => void;
  hasExpectedBinaries?: boolean;
}) => {
  const rows =
    kind === "changed" && unexpectedOnly
      ? set.files.filter((f) => !isExpectedBinaryOnly(f))
      : set.files;
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
      {kind === "changed" && hasExpectedBinaries && onToggleUnexpectedOnly && (
        <div className="diffFilesToolbar">
          <Checkbox
            checked={unexpectedOnly}
            onChange={(e) => onToggleUnexpectedOnly(e.target.checked)}
          >
            {i18nGet("diffUnexpectedOnly")}
          </Checkbox>
        </div>
      )}
      {rows.length === 0 ? (
        <p className="diffEmpty">{i18nGet("diffNoFiles")}</p>
      ) : (
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
            {rows.map((f) => (
              <tr key={f.path}>
                <td>
                  <code>{f.path}</code>
                  {kind === "changed" && <FileBadges f={f} />}
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
      )}
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
  const [unexpectedOnly, setUnexpectedOnly] = useState(false);
  const tab = (
    key: "added" | "removed" | "changed",
    label: string,
    set: ChangeSet,
  ) => ({
    key,
    label: `${label} · ${set.count.toLocaleString()} · ${signedBytes(set.bytes)}`,
    children: (
      <FileTable
        set={set}
        kind={key}
        unexpectedOnly={unexpectedOnly}
        onToggleUnexpectedOnly={setUnexpectedOnly}
        hasExpectedBinaries={diff.parity.expectedBinaryChanges > 0}
      />
    ),
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
  ({
    diff,
    refs,
    baseMode,
    onChangeBaseMode,
    onSwap,
    onOpen,
  }: DiffActions & { diff: ImageDiff }) => {
    const hasRecs =
      diff.newRecommendations.length > 0 ||
      diff.resolvedRecommendations.length > 0;
    return (
      <>
        <DiffHeader
          diff={diff}
          refs={refs}
          baseMode={baseMode}
          onChangeBaseMode={onChangeBaseMode}
          onSwap={onSwap}
          onOpen={onOpen}
        />
        <DiffConfigRuntime diff={diff} />
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
