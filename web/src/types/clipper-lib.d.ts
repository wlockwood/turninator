declare module 'clipper-lib' {
  namespace ClipperLib {
    interface IntPoint {
      X: number;
      Y: number;
    }
    type Path = IntPoint[];
    type Paths = Path[];
    enum ClipType { ctIntersection = 0, ctUnion = 1, ctDifference = 2, ctXor = 3 }
    enum PolyType { ptSubject = 0, ptClip = 1 }
    enum PolyFillType { pftEvenOdd = 0, pftNonZero = 1, pftPositive = 2, pftNegative = 3 }
    enum JoinType { jtSquare = 0, jtRound = 1, jtMiter = 2 }
    enum EndType { etOpenSquare = 0, etOpenRound = 1, etOpenButt = 2, etClosedLine = 3, etClosedPolygon = 4 }
    class ClipperOffset {
      constructor(miterLimit?: number, arcTolerance?: number);
      AddPaths(paths: Paths, joinType: JoinType, endType: EndType): void;
      Execute(solution: Paths, delta: number): void;
    }
    class PolyNode {
      Contour(): Path;
      Childs(): PolyNode[];
      IsHole(): boolean;
      ChildCount(): number;
    }
    class PolyTree extends PolyNode {
      Total(): number;
    }
    class Clipper {
      constructor(initOptions?: number);
      AddPath(path: Path, polyType: PolyType, closed: boolean): boolean;
      AddPaths(paths: Paths, polyType: PolyType, closed: boolean): boolean;
      Execute(clipType: ClipType, solution: Paths | PolyTree, subjFillType?: PolyFillType, clipFillType?: PolyFillType): boolean;
      Clear(): void;
      StrictlySimple: boolean;
      static CleanPolygons(paths: Paths, distance?: number): Paths;
      static SimplifyPolygons(paths: Paths, fillType?: PolyFillType): Paths;
      static Area(path: Path): number;
      static Orientation(path: Path): boolean;
    }
  }
  export = ClipperLib;
}
