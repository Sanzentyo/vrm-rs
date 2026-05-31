// @ts-types="https://sanzentyo.github.io/imq/imqraw/v0.1.0/imqraw.d.ts"
import {
  encodeBundle,
  imqraw_image_count,
  init,
  type Rgba8Image,
} from "https://sanzentyo.github.io/imq/imqraw/v0.1.0/imqraw.js";

type Options = {
  expected: string;
  actual: string;
  metrics: string;
  format: string;
  output?: string;
};

type RgbaArtifact = {
  width: number;
  height: number;
  rgba: number[];
};

const defaultMetrics =
  "psnr:color,mse:color,mae:color,maxae:color,psnr:all,mse:all";

const parseOptions = (args: string[]): Options => {
  const options: Partial<Options> = {
    metrics: defaultMetrics,
    format: "json",
  };

  for (let i = 0; i < args.length; i += 1) {
    const name = args[i];
    if (name === "--help" || name === "-h") {
      printHelp();
      Deno.exit(0);
    }
    if (!name.startsWith("--")) {
      throw new Error(`unexpected positional argument: ${name}`);
    }
    const value = args[i + 1];
    if (value === undefined || value.startsWith("--")) {
      throw new Error(`${name} requires a value`);
    }
    i += 1;

    switch (name) {
      case "--expected":
        options.expected = value;
        break;
      case "--actual":
        options.actual = value;
        break;
      case "--metrics":
        options.metrics = value;
        break;
      case "--format":
        options.format = value;
        break;
      case "--output":
        options.output = value;
        break;
      default:
        throw new Error(`unknown option: ${name}`);
    }
  }

  if (options.expected === undefined || options.actual === undefined) {
    throw new Error("--expected and --actual are required");
  }

  return options as Options;
};

const printHelp = () => {
  console.log(`Usage:
  deno run --allow-import=sanzentyo.github.io --allow-net=sanzentyo.github.io --allow-read --allow-run=imq --allow-write tools/render-parity/imqraw-compare-rgba-json.ts --expected REF.rgba.json --actual CANDIDATE.rgba.json --output report.json

Options:
  --expected PATH   Reference render-parity .rgba.json artifact
  --actual PATH     Candidate render-parity .rgba.json artifact
  --metrics LIST    imq metric list [default: ${defaultMetrics}]
  --format FORMAT   imq output format [default: json]
  --output PATH     Optional report path written by imq`);
};

const readArtifact = async (path: string): Promise<RgbaArtifact> => {
  const artifact = JSON.parse(await Deno.readTextFile(path)) as RgbaArtifact;
  const expectedLength = checkedRgbaLength(artifact.width, artifact.height);
  if (artifact.rgba.length !== expectedLength) {
    throw new Error(
      `${path}: rgba length ${artifact.rgba.length} does not match expected ${expectedLength}`,
    );
  }
  return artifact;
};

const checkedRgbaLength = (width: number, height: number): number => {
  if (
    !Number.isSafeInteger(width) || !Number.isSafeInteger(height) ||
    width <= 0 || height <= 0
  ) {
    throw new Error(`invalid dimensions: ${width}x${height}`);
  }
  const bytes = width * height * 4;
  if (!Number.isSafeInteger(bytes)) {
    throw new Error(`RGBA dimensions overflow: ${width}x${height}`);
  }
  return bytes;
};

const toImage = (
  artifact: RgbaArtifact,
  label: string,
  tags: string[],
): Rgba8Image => ({
  data: Uint8Array.from(artifact.rgba),
  width: artifact.width,
  height: artifact.height,
  label,
  tags,
});

const compare = async (options: Options) => {
  await init();

  const expected = await readArtifact(options.expected);
  const actual = await readArtifact(options.actual);
  if (expected.width !== actual.width || expected.height !== actual.height) {
    throw new Error(
      `image dimensions differ: expected ${expected.width}x${expected.height}, actual ${actual.width}x${actual.height}`,
    );
  }

  const bundle = encodeBundle([
    toImage(expected, "reference", ["reference"]),
    toImage(actual, "candidate", ["candidate"]),
  ]);
  const imageCount = imqraw_image_count(bundle);
  if (imageCount !== 2) {
    throw new Error(`encoded imqraw bundle contains ${imageCount} images`);
  }

  const commandArgs = [
    "image",
    "-",
    "-",
    "--stdin-format",
    "imqraw",
    "--stdin-reference-tag",
    "reference",
    "--stdin-distorted-tag",
    "candidate",
    "--metrics",
    options.metrics,
    "--format",
    options.format,
  ];
  if (options.output !== undefined) {
    const separator = Math.max(
      options.output.lastIndexOf("/"),
      options.output.lastIndexOf("\\"),
    );
    if (separator > 0) {
      const parent = options.output.slice(0, separator);
      await Deno.mkdir(parent, { recursive: true });
    }
    commandArgs.push("--output", options.output);
  }

  const command = new Deno.Command("imq", {
    args: commandArgs,
    stdin: "piped",
    stdout: "inherit",
    stderr: "inherit",
  });
  const child = command.spawn();
  const writer = child.stdin.getWriter();
  await writer.write(bundle);
  await writer.close();

  const status = await child.status;
  if (!status.success) {
    throw new Error(`imq exited with code ${status.code}`);
  }
};

if (import.meta.main) {
  await compare(parseOptions(Deno.args));
}
