use super::value::Val;

/// The values of one variable over the rows of a frame.
#[derive(Clone, Debug, Default)]
pub enum Col {
    #[default]
    Unset,
    /// The same value at every row.
    One(Val),
    Many(Vec<Val>),
}

/// Rows of bound variables, one column per variable of the plan.
///
/// A column holds a value at the rows where its variable is bound; reading it elsewhere is a
/// broken invariant of the plan.
#[derive(Clone, Debug)]
pub struct Frame {
    len: usize,
    cols: Vec<Col>,
}

/// Row positions of a frame, ascending.
pub type Sel = Vec<u32>;

impl Frame {
    /// Returns a frame of one row with no variable bound.
    pub fn unit(vars: usize) -> Frame {
        Frame {
            len: 1,
            cols: vec![Col::Unset; vars],
        }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn all(&self) -> Sel {
        (0..self.len as u32).collect()
    }

    pub fn get(&self, var: u32, row: u32) -> &Val {
        match &self.cols[var as usize] {
            Col::One(v) => v,
            Col::Many(vs) => &vs[row as usize],
            Col::Unset => panic!("variable {var} is read where it is not bound"),
        }
    }

    pub fn bind_one(&mut self, var: u32, v: Val) {
        self.cols[var as usize] = Col::One(v);
    }

    /// Bind `var` at each row of `values`.
    pub fn bind(&mut self, var: u32, values: impl IntoIterator<Item = (u32, Val)>) {
        let len = self.len;
        let col = &mut self.cols[var as usize];
        let vs = match col {
            Col::Many(vs) => vs,
            Col::One(v) => {
                *col = Col::Many(vec![v.clone(); len]);
                let Col::Many(vs) = col else { unreachable!() };
                vs
            }
            Col::Unset => {
                *col = Col::Many(vec![Val::Int(0); len]);
                let Col::Many(vs) = col else { unreachable!() };
                vs
            }
        };
        for (r, v) in values {
            vs[r as usize] = v;
        }
    }

    /// Bind `var` at every row to the values of `values`, one per row.
    pub fn bind_all(&mut self, var: u32, values: Vec<Val>) {
        assert_eq!(self.len, values.len(), "one value per row");
        self.cols[var as usize] = Col::Many(values);
    }

    /// Returns a frame whose rows are the rows `from` names, in order.
    pub fn gather(&self, from: &[u32]) -> Frame {
        let cols = self
            .cols
            .iter()
            .map(|c| match c {
                Col::Many(vs) => Col::Many(from.iter().map(|&r| vs[r as usize].clone()).collect()),
                other => other.clone(),
            })
            .collect();
        Frame {
            len: from.len(),
            cols,
        }
    }

    /// Returns the one-row frame of row `row`.
    pub fn row(&self, row: u32) -> Frame {
        let cols = self
            .cols
            .iter()
            .map(|c| match c {
                Col::Many(vs) => Col::One(vs[row as usize].clone()),
                other => other.clone(),
            })
            .collect();
        Frame { len: 1, cols }
    }

    /// Returns a frame of `len` rows that binds every variable this one-row frame binds.
    pub fn widen(&self, len: usize) -> Frame {
        assert_eq!(1, self.len, "only a one-row frame widens");
        let cols = self
            .cols
            .iter()
            .map(|c| match c {
                Col::Many(vs) => Col::One(vs[0].clone()),
                other => other.clone(),
            })
            .collect();
        Frame { len, cols }
    }

    /// Returns an estimate of the bytes the frame's own columns hold.
    pub fn size(&self) -> usize {
        self.cols
            .iter()
            .map(|c| match c {
                Col::Many(vs) => vs.iter().map(Val::size).sum(),
                _ => 0,
            })
            .sum()
    }
}
