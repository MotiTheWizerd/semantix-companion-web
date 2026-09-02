// d3-force-3d ships no types. This is the slice the memory sky uses — the
// d3-force API with a third dimension; every accessor is chainable.

declare module "d3-force-3d" {
  export interface SimulationNode {
    index?: number;
    x?: number;
    y?: number;
    z?: number;
    vx?: number;
    vy?: number;
    vz?: number;
    fx?: number | null;
    fy?: number | null;
    fz?: number | null;
  }

  export interface SimulationLink<N extends SimulationNode = SimulationNode> {
    source: N | number | string;
    target: N | number | string;
    index?: number;
  }

  export interface Force<N extends SimulationNode = SimulationNode> {
    (alpha: number): void;
    initialize?(nodes: N[], random: () => number, numDimensions: number): void;
  }

  export interface Simulation<N extends SimulationNode = SimulationNode> {
    tick(iterations?: number): this;
    restart(): this;
    stop(): this;
    nodes(): N[];
    nodes(nodes: N[]): this;
    alpha(): number;
    alpha(alpha: number): this;
    alphaMin(): number;
    alphaMin(min: number): this;
    alphaDecay(): number;
    alphaDecay(decay: number): this;
    alphaTarget(): number;
    alphaTarget(target: number): this;
    velocityDecay(): number;
    velocityDecay(decay: number): this;
    force(name: string): Force<N> | undefined;
    force(name: string, force: Force<N> | null): this;
    numDimensions(): number;
    numDimensions(n: number): this;
  }

  export interface LinkForce<N extends SimulationNode = SimulationNode> extends Force<N> {
    links(): SimulationLink<N>[];
    links(links: SimulationLink<N>[]): this;
    id(accessor: (node: N, index: number) => string | number): this;
    distance(distance: number | ((link: SimulationLink<N>, index: number) => number)): this;
    strength(strength: number | ((link: SimulationLink<N>, index: number) => number)): this;
    iterations(iterations: number): this;
  }

  export interface ManyBodyForce<N extends SimulationNode = SimulationNode> extends Force<N> {
    strength(strength: number | ((node: N, index: number) => number)): this;
    theta(theta: number): this;
    distanceMin(min: number): this;
    distanceMax(max: number): this;
  }

  export interface PositioningForce<N extends SimulationNode = SimulationNode> extends Force<N> {
    strength(strength: number | ((node: N, index: number) => number)): this;
    x?(x: number | ((node: N, index: number) => number)): this;
    y?(y: number | ((node: N, index: number) => number)): this;
    z?(z: number | ((node: N, index: number) => number)): this;
  }

  export interface CollideForce<N extends SimulationNode = SimulationNode> extends Force<N> {
    radius(radius: number | ((node: N, index: number) => number)): this;
    strength(strength: number): this;
    iterations(iterations: number): this;
  }

  export function forceSimulation<N extends SimulationNode = SimulationNode>(
    nodes?: N[],
    numDimensions?: number,
  ): Simulation<N>;
  export function forceLink<N extends SimulationNode = SimulationNode>(
    links?: SimulationLink<N>[],
  ): LinkForce<N>;
  export function forceManyBody<N extends SimulationNode = SimulationNode>(): ManyBodyForce<N>;
  export function forceCenter<N extends SimulationNode = SimulationNode>(
    x?: number,
    y?: number,
    z?: number,
  ): Force<N> & { strength(strength: number): Force<N> };
  export function forceCollide<N extends SimulationNode = SimulationNode>(
    radius?: number | ((node: N) => number),
  ): CollideForce<N>;
  export function forceX<N extends SimulationNode = SimulationNode>(
    x?: number | ((node: N, index: number) => number),
  ): PositioningForce<N>;
  export function forceY<N extends SimulationNode = SimulationNode>(
    y?: number | ((node: N, index: number) => number),
  ): PositioningForce<N>;
  export function forceZ<N extends SimulationNode = SimulationNode>(
    z?: number | ((node: N, index: number) => number),
  ): PositioningForce<N>;
  export function forceRadial<N extends SimulationNode = SimulationNode>(
    radius: number | ((node: N, index: number) => number),
    x?: number,
    y?: number,
    z?: number,
  ): PositioningForce<N>;
}
