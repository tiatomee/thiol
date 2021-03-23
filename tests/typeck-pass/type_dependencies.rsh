type
    A = record end
    B = A;
    C = Option<B>;

    // just an annotation for now
    Pointer<T> = T;

    Option<T> = record
        is_some: bool;
        value: Pointer<T>;
    end

    Pair<A, B> = record
        first: A;
        second: B;
    end

    Vertex = record
        position: float4 is Point in ObjectSpace;
        colour : float4 is Colour in Linear;
        uv : float2 is Vector in UV;
        additional : Pair<bool, bool>;
    end

// args: --dump-type-context
//
// expected stdout:
// A = (0) record  end
// B = (0) record  end
// C = (2) record
//     is_some : (1) bool
//     value : (0) record  end
// end
// Vertex = (7) record
//     position : (3) float4[Point]{ObjectSpace}
//     colour : (4) float4[Colour]{Linear}
//     uv : (5) float2[Vector]{UV}
//     additional : (6) record
//         first : (1) bool
//         second : (1) bool
//     end
// end