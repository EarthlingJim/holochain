use once_cell::sync::OnceCell;

use crate::{
    arq::*,
    error::{GossipError, GossipResult},
    op::OpRegion,
    persistence::AccessOpStore,
    quantum::*,
    tree::TreeDataConstraints,
};
use derivative::Derivative;

use super::{Region, RegionBounds, RegionCoords, RegionData};

#[derive(Debug, PartialEq, Eq, derive_more::Constructor, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "test_utils", derive(Clone))]
pub struct RegionCoordSetXtcs {
    times: TelescopingTimes,
    arq_set: ArqBoundsSet,
}

impl RegionCoordSetXtcs {
    /// Generate the XTCS region coords given the generating parameters.
    /// Each RegionCoords is paired with the relative spacetime coords, which
    /// can be used to pair the generated coords with stored data.
    pub fn region_coords_flat<'a>(
        &'a self,
    ) -> impl Iterator<Item = ((u32, u32), RegionCoords)> + 'a {
        self.region_coords_nested().flatten()
    }

    pub fn region_coords_nested<'a>(
        &'a self,
    ) -> impl Iterator<Item = impl Iterator<Item = ((u32, u32), RegionCoords)>> + 'a {
        self.arq_set.arqs().iter().flat_map(move |arq| {
            arq.segments().enumerate().map(move |(ix, x)| {
                self.times
                    .segments()
                    .into_iter()
                    .enumerate()
                    .map(move |(it, t)| ((ix as u32, it as u32), RegionCoords::new(x, t)))
            })
        })
    }

    pub fn into_region_set<D, E, F>(self, mut f: F) -> Result<RegionSetXtcs<D>, E>
    where
        D: TreeDataConstraints,
        F: FnMut(((u32, u32), RegionCoords)) -> Result<D, E>,
    {
        let data = self
            .region_coords_nested()
            .map(move |column| column.map(&mut f).collect::<Result<Vec<D>, E>>())
            .collect::<Result<Vec<Vec<D>>, E>>()?;
        Ok(RegionSetXtcs::from_data(self, data))
    }

    pub fn empty() -> Self {
        Self {
            times: TelescopingTimes::empty(),
            arq_set: ArqBoundsSet::empty(),
        }
    }
}

/// The generic definition of a set of Regions.
/// The current representation is very specific to our current algorithm,
/// but this is an enum to make room for a more generic representation, e.g.
/// a simple Vec<Region>, if we want a more intricate algorithm later.
#[derive(Debug, derive_more::From)]
#[cfg_attr(feature = "test_utils", derive(Clone))]
pub enum RegionSet<T: TreeDataConstraints = RegionData> {
    /// eXponential Time, Constant Space.
    Xtcs(RegionSetXtcs<T>),
}

impl<D: TreeDataConstraints> RegionSet<D> {
    pub fn count(&self) -> usize {
        match self {
            Self::Xtcs(set) => set.count(),
        }
    }

    /// can be used to pair the generated coords with stored data.
    pub fn region_coords<'a>(&'a self) -> impl Iterator<Item = RegionCoords> + 'a {
        match self {
            Self::Xtcs(set) => set.coords.region_coords_flat().map(|(_, coords)| coords),
        }
    }

    pub fn regions<'a>(&'a self) -> impl Iterator<Item = Region<D>> + 'a {
        match self {
            Self::Xtcs(set) => set.regions(),
        }
    }

    pub fn query(&self, _bounds: &RegionBounds) -> ! {
        unimplemented!("only implement after trying naive database-only approach")
    }

    pub fn update(&self, _c: SpacetimeCoords, _d: D) -> ! {
        unimplemented!("only implement after trying naive database-only approach")
    }

    /// Find a set of Regions which represents the intersection of the two
    /// input RegionSets.
    pub fn diff(self, other: Self) -> GossipResult<Vec<Region<D>>> {
        match (self, other) {
            (Self::Xtcs(left), Self::Xtcs(right)) => left.diff(right),
        }
        // Notes on a generic algorithm for the diff of generic regions:
        // can we use a Fenwick tree to look up regions?
        // idea:
        // sort the regions by power (problem, there are two power)
        // lookup the region to see if there's already a direct hit (most efficient if the sorting guarantees that larger regions get looked up later)
        // PROBLEM: we *can't* resolve rectangles where one is not a subset of the other
    }
}

/// Implementation for the compact XTCS region set format which gets sent over the wire.
/// The coordinates for the regions are specified by a few values.
/// The data to match the coordinates are specified in a 2D vector which must
/// correspond to the generated coordinates.
#[derive(Debug, serde::Serialize, serde::Deserialize, Derivative)]
#[derivative(PartialEq, Eq)]
#[cfg_attr(feature = "test_utils", derive(Clone))]
pub struct RegionSetXtcs<D: TreeDataConstraints = RegionData> {
    /// The generator for the coordinates
    pub(crate) coords: RegionCoordSetXtcs,

    /// the actual coordinates as generated
    #[derivative(PartialEq = "ignore")]
    #[serde(skip)]
    pub(crate) _region_coords: OnceCell<Vec<RegionCoords>>,

    /// The outer vec corresponds to the spatial segments;
    /// the inner vecs are the time segments.
    #[serde(bound(deserialize = "D: serde::de::DeserializeOwned"))]
    pub(crate) data: Vec<Vec<D>>,
}

impl<D: TreeDataConstraints> RegionSetXtcs<D> {
    pub fn empty() -> Self {
        Self {
            coords: RegionCoordSetXtcs::empty(),
            data: vec![],
            _region_coords: OnceCell::new(),
        }
    }

    pub fn from_data(coords: RegionCoordSetXtcs, data: Vec<Vec<D>>) -> Self {
        Self {
            coords,
            data,
            _region_coords: OnceCell::new(),
        }
    }

    pub fn from_store<O: OpRegion<D>, S: AccessOpStore<D, O>>(
        store: &S,
        coords: RegionCoordSetXtcs,
    ) -> Self {
        let data = coords
            .region_coords_nested()
            .map(|columns| {
                columns
                    .map(|(_, coords)| store.query_region_coords(&coords))
                    .collect()
            })
            .collect();
        Self {
            coords,
            data,
            _region_coords: OnceCell::new(),
        }
    }

    pub fn count(&self) -> usize {
        if self.data.is_empty() {
            0
        } else {
            self.data.len() * self.data[0].len()
        }
    }

    pub fn regions<'a>(&'a self) -> impl Iterator<Item = Region<D>> + 'a {
        self.coords
            .region_coords_flat()
            .map(|((ix, it), coords)| Region::new(coords, self.data[ix as usize][it as usize]))
    }

    /// Reshape the two region sets so that both match, omitting or merging
    /// regions as needed
    pub fn rectify(&mut self, other: &mut Self) -> GossipResult<()> {
        if self.coords.arq_set != other.coords.arq_set {
            return Err(GossipError::ArqSetMismatchForDiff);
        }
        if self.coords.times > other.coords.times {
            std::mem::swap(self, other);
        }
        let mut len = 0;
        for (da, db) in self.data.iter_mut().zip(other.data.iter_mut()) {
            TelescopingTimes::rectify((&self.coords.times, da), (&other.coords.times, db));
            len = da.len();
        }
        let times = other.coords.times.limit(len as u32);
        self.coords.times = times;
        other.coords.times = times;
        Ok(())
    }

    pub fn diff(mut self, mut other: Self) -> GossipResult<Vec<Region<D>>> {
        self.rectify(&mut other)?;

        let regions = self
            .regions()
            .into_iter()
            .zip(other.regions().into_iter())
            .filter_map(|(a, b)| (a.data != b.data).then(|| a))
            .collect();

        Ok(regions)
    }
}

#[cfg(test)]
#[cfg(feature = "test_utils")]
mod tests {

    use kitsune_p2p_timestamp::Timestamp;

    use crate::{
        op::{Op, OpData},
        test_utils::op_store::OpStore,
        Loc,
    };

    use super::*;

    /// Create a uniform grid of ops:
    /// - one gridline per arq segment
    /// - one gridline per time specified in the iterator
    ///
    /// Only works for arqs that don't span `u32::MAX / 2`
    fn op_grid<S: ArqStart>(
        topo: &Topology,
        arq: &Arq<S>,
        trange: impl Iterator<Item = i64> + Clone,
    ) -> Vec<Op> {
        let (left, right) = arq.to_edge_locs(topo);
        let left = left.as_u32();
        let right = right.as_u32();
        let mid = u32::MAX / 2;
        assert!(
            !(left < mid && right > mid),
            "This hacky logic does not work for arqs which span `u32::MAX / 2`"
        );
        let xstep = (arq.absolute_length(topo) / arq.count() as u64) as usize;
        (left as i32..=right as i32)
            .step_by(xstep)
            .flat_map(|x| {
                trange.clone().map(move |t| {
                    let x = SpaceQuantum::from(x as u32).to_loc_bounds(topo).0;
                    let t = TimeQuantum::from(t as u32).to_timestamp_bounds(topo).0;
                    OpData::fake(x, t, 10)
                })
            })
            .collect()
    }

    #[test]
    fn test_regions() {
        let topo = Topology::unit(Timestamp::from_micros(1000));
        let pow = 8;
        let arq = Arq::new(0u32.into(), pow, 4);
        assert_eq!(
            arq.to_edge_locs(&topo),
            (Loc::from(0u32), Loc::from(1023u32))
        );

        let mut store = OpStore::new(topo.clone(), GossipParams::zero());

        // Create a nx by nt grid of ops and integrate into the store
        let nx = 8;
        let nt = 10;
        let ops = op_grid(
            &topo,
            &Arq::new(0u32.into(), pow, 8),
            (1000..11000 as i64).step_by(1000),
        );
        assert_eq!(ops.len(), nx * nt);
        store.integrate_ops(ops.into_iter());

        // Calculate region data for all ops.
        // The total count should be half of what's in the op store,
        // since the arq covers exactly half of the ops
        let times = TelescopingTimes::new(TimeQuantum::from(11000));
        let coords = RegionCoordSetXtcs::new(times, ArqBoundsSet::single(arq.to_bounds(&topo)));
        let rset = RegionSetXtcs::from_store(&store, coords);
        assert_eq!(
            rset.data.concat().iter().map(|r| r.count).sum::<u32>() as usize,
            nx * nt / 2
        );
    }

    #[test]
    fn test_rectify() {
        let topo = Topology::unit_zero();
        let arq = Arq::new(0u32.into(), 8, 4).to_bounds(&topo);
        let mut store = OpStore::new(topo.clone(), GossipParams::zero());
        store.integrate_ops(op_grid(&topo, &arq, 10..20).into_iter());

        let tt_a = TelescopingTimes::new(TimeQuantum::from(20));
        let tt_b = TelescopingTimes::new(TimeQuantum::from(30));
        let coords_a = RegionCoordSetXtcs::new(tt_a, ArqBoundsSet::single(arq.clone()));
        let coords_b = RegionCoordSetXtcs::new(tt_b, ArqBoundsSet::single(arq.clone()));

        let mut rset_a = RegionSetXtcs::from_store(&store, coords_a);
        let mut rset_b = RegionSetXtcs::from_store(&store, coords_b);
        assert_ne!(rset_a.data, rset_b.data);

        rset_a.rectify(&mut rset_b).unwrap();

        assert_eq!(rset_a, rset_b);

        let coords: Vec<Vec<_>> = rset_a
            .coords
            .region_coords_nested()
            .map(|col| col.collect())
            .collect();

        assert_eq!(coords.len(), arq.count() as usize);
        for col in coords.iter() {
            assert_eq!(col.len(), rset_a.coords.times.segments().len());
        }
        let nt = coords[0].len();
        assert_eq!(tt_b.segments()[0..nt], rset_a.coords.times.segments());
        assert_eq!(tt_b.segments()[0..nt], rset_b.coords.times.segments());
    }

    #[test]
    fn test_diff() {
        let topo = Topology::unit_zero();
        let arq = Arq::new(Loc::from(-512i32 as u32), 8, 4).to_bounds(&topo);
        dbg!(&arq, arq.to_interval(&topo));

        let mut store1 = OpStore::new(topo.clone(), GossipParams::zero());
        store1.integrate_ops(op_grid(&topo, &arq, 10..20).into_iter());

        let extra_ops = [
            OpData::fake(Loc::from(-300i32), Timestamp::from_micros(18), 4),
            OpData::fake(Loc::from(12u32), Timestamp::from_micros(12), 4),
        ];
        let mut store2 = store1.clone();
        store2.integrate_ops(extra_ops.clone().into_iter());

        let coords_a = RegionCoordSetXtcs::new(
            TelescopingTimes::new(TimeQuantum::from(20)),
            ArqBoundsSet::single(arq.clone()),
        );
        let coords_b = RegionCoordSetXtcs::new(
            TelescopingTimes::new(TimeQuantum::from(21)),
            ArqBoundsSet::single(arq.clone()),
        );

        let rset_a = RegionSetXtcs::from_store(&store1, coords_a);
        let rset_b = RegionSetXtcs::from_store(&store2, coords_b);
        assert_ne!(rset_a.data, rset_b.data);

        let diff = rset_a.clone().diff(rset_b.clone()).unwrap();
        dbg!(&diff, &extra_ops);
        assert_eq!(diff.len(), 2);

        assert!(diff[0].coords.contains(&topo, &extra_ops[0].coords(&topo)));
        assert!(diff[1].coords.contains(&topo, &extra_ops[1].coords(&topo)));

        // Adding the region data from each extra op to the region data of the
        // diff which was missing those ops should be the same as the query
        // of the store which contains the extra ops over the same region
        // TODO: proptest this
        assert_eq!(
            diff[0].data + extra_ops[0].region_data(),
            store2.query_region_coords(&diff[0].coords)
        );
        assert_eq!(
            diff[1].data + extra_ops[1].region_data(),
            store2.query_region_coords(&diff[1].coords)
        );
    }

    #[test]
    fn test_diff_standard_topo() {
        let topo = Topology::standard_zero();
        let pow: u8 = 4;
        // This arq goes from -2^17 to 2^17, with a chunk size of 2^16
        let left_edge = Loc::from(-(2i32.pow(pow as u32 + 12 + 1)));
        let arq = Arq::new(left_edge, pow, 4).to_bounds(&topo);
        dbg!(&arq, arq.to_interval(&topo));

        let mut store1 = OpStore::new(topo.clone(), GossipParams::zero());
        store1.integrate_ops(op_grid(&topo, &arq, 10..20).into_iter());

        let extra_ops = [
            OpData::fake(
                left_edge,
                TimeQuantum::from(18).to_timestamp_bounds(&topo).0,
                13,
            ),
            OpData::fake(
                Loc::from(11111u32),
                TimeQuantum::from(12).to_timestamp_bounds(&topo).0,
                11,
            ),
        ];
        // Store 2 has everything store 1 has, plus 2 extra ops
        let mut store2 = store1.clone();
        store2.integrate_ops(extra_ops.clone().into_iter());

        let coords_a = RegionCoordSetXtcs::new(
            TelescopingTimes::new(TimeQuantum::from(20)),
            ArqBoundsSet::single(arq.clone()),
        );
        let coords_b = RegionCoordSetXtcs::new(
            TelescopingTimes::new(TimeQuantum::from(21)),
            ArqBoundsSet::single(arq.clone()),
        );

        let rset_a = RegionSetXtcs::from_store(&store1, coords_a);
        let rset_b = RegionSetXtcs::from_store(&store2, coords_b);
        assert_ne!(rset_a.data, rset_b.data);

        let diff = rset_a.clone().diff(rset_b.clone()).unwrap();
        dbg!(&diff, &extra_ops);
        assert_eq!(diff.len(), 2);

        assert!(diff[0].coords.contains(&topo, &extra_ops[0].coords(&topo)));
        assert!(diff[1].coords.contains(&topo, &extra_ops[1].coords(&topo)));

        // Adding the region data from each extra op to the region data of the
        // diff which was missing those ops should be the same as the query
        // of the store which contains the extra ops over the same region
        // TODO: proptest this
        assert_eq!(
            diff[0].data + extra_ops[0].region_data(),
            store2.query_region_coords(&diff[0].coords)
        );
        assert_eq!(
            diff[1].data + extra_ops[1].region_data(),
            store2.query_region_coords(&diff[1].coords)
        );
    }
}
