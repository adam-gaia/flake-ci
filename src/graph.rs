use anyhow::{bail, Result};
use owo_colors::OwoColorize;
use std::cmp::Eq;
use std::fmt::{Debug, Display};
use std::ops::IndexMut;
use std::{collections::HashMap, hash::Hash};

#[derive(Debug)]
pub struct Graph<T> {
    nodes: Vec<T>,
    children: Vec<Vec<usize>>,
    parents: Vec<Option<usize>>,
}

impl<T> Graph<T>
where
    T: Hash + Eq + PartialEq + Debug + Clone,
{
    pub fn new() -> Self {
        Self {
            nodes: Vec::new(),
            children: Vec::new(),
            parents: Vec::new(),
        }
    }

    pub fn add_node(&mut self, data: T) {
        self.nodes.push(data);
        self.children.push(Vec::new());
        self.parents.push(None);
    }

    fn get_index_of(&self, data: &T) -> Option<usize> {
        self.nodes.iter().position(|x| x == data)
    }

    pub fn mark_dep(&mut self, parent: &T, child: &T) -> Result<()> {
        let Some(parent_index) = self.get_index_of(parent) else {
            bail!("Parent {parent:?} not in graph");
        };

        let Some(child_index) = self.get_index_of(child) else {
            bail!("Child {child:?} not in graph");
        };

        let Some(v) = self.children.get_mut(parent_index) else {
            bail!("Graph not set up for parent {parent:?}");
        };

        v.push(child_index);
        self.parents[child_index] = Some(parent_index);

        // Make sure we haven't built a circle
        // TODO: validate this with some unit tests
        let mut child_index = child_index;
        let mut path = vec![child_index];
        while let Some(parent_index) = self.parents[child_index] {
            path.push(parent_index);
            if parent_index == child_index {
                bail!("Circular graph: {path:?}")
            }
            child_index = parent_index;
        }

        Ok(())
    }

    pub fn walker(self) -> GraphWalker<T> {
        GraphWalker::new(self)
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    fn is_leaf(&self, idx: usize) -> bool {
        self.children[idx].is_empty()
    }

    fn data_of(&self, idx: usize) -> T {
        self.nodes[idx].clone()
    }

    fn parent_of(&self, idx: usize) -> Option<usize> {
        self.parents[idx]
    }
}

#[derive(Debug, PartialEq, Eq, Clone)]
enum Status {
    NotWalked,
    Walked,
}

#[derive(Debug)]
pub struct GraphWalker<T> {
    size: usize,
    walked: Vec<Status>,
    graph: Graph<T>,
}

impl<T> GraphWalker<T>
where
    T: Hash + PartialEq + Eq + Debug + Clone,
{
    pub fn new(graph: Graph<T>) -> Self {
        let n = graph.len();
        let walked = vec![Status::NotWalked; n];
        Self {
            walked,
            graph,
            size: n,
        }
    }

    fn walked(&self, index: usize) -> bool {
        self.walked[index] != Status::NotWalked
    }

    pub fn chains(mut self) -> Vec<Vec<T>> {
        let mut chains = Vec::new();
        let mut current_chain = Vec::new();

        // Find the first unwalked node node
        let num_nodes = self.size;
        for current in 0..num_nodes {
            if self.walked(current) {
                continue;
            }

            if self.graph.is_leaf(current) {
                let mut current = current;
                let data = self.graph.data_of(current);
                current_chain.push(data);
                self.walked[current] = Status::Walked;
                while let Some(parent) = self.graph.parent_of(current) {
                    let parent_data = self.graph.data_of(parent);
                    current_chain.push(parent_data);
                    self.walked[parent] = Status::Walked;
                    current = parent;
                }
                current_chain.reverse();
                chains.push(current_chain);
                current_chain = Vec::new();
            }
        }
        chains
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    // TODO: test that we can't have a circle

    // TODO: test this backwards
    #[test]
    fn test_simple_chain() {
        // parent -> child
        // 0 -> 1 -> 2 -> 3 ... -> 9
        // Will get built in order

        let n = 10;
        let mut expected = Vec::new();
        let mut g = Graph::new();
        for i in 0..n {
            let data = format!("{i}");
            expected.push(data.clone());
            g.add_node(data);
        }

        for i in 0..(n - 1) {
            let parent = i;
            let child = parent + 1;
            g.mark_dep(&format!("{parent}"), &format!("{child}"))
                .unwrap();
        }

        let walker = g.walker();
        let actual = walker.chains();

        assert_eq!(vec![expected], actual);
    }

    #[test]
    fn test_one_parent() {
        // parent -> child
        // 9 -> 0
        // 9 -> 1
        // 9 -> 2
        // ...
        // 9 -> 8
        // 9 is the parent of them all, so 9 will get built first, then the others

        let n = 10;
        let last = n - 1;

        let mut g = Graph::new();
        for i in 0..n {
            let data = format!("{i}");
            g.add_node(data);
        }

        for i in 0..(n - 1) {
            let parent = last;
            let child = i;
            g.mark_dep(&format!("{parent}"), &format!("{child}"))
                .unwrap();
        }

        let walker = g.walker();
        let actual = walker.chains();

        let mut expected = Vec::new();
        for i in 0..(n - 1) {
            expected.push(vec![format!("{last}"), format!("{i}")]);
        }
        assert_eq!(expected, actual);
    }
}
