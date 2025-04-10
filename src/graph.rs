use anyhow::{bail, Result};
use log::debug;
use owo_colors::OwoColorize;
use std::cmp::Eq;
use std::collections::HashSet;
use std::fmt::{Debug, Display};
use std::ops::IndexMut;
use std::{collections::HashMap, hash::Hash};

#[derive(Debug)]
pub struct Graph<T> {
    nodes: Vec<T>,
    children: Vec<Vec<usize>>,
    parents: Vec<Vec<usize>>,
}

impl<T> Graph<T>
where
    T: Hash + Eq + PartialEq + Debug + Clone + Display,
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
        self.parents.push(Vec::new());
    }

    fn get_index_of(&self, data: &T) -> Option<usize> {
        self.nodes.iter().position(|x| x == data)
    }

    fn rec_check(&self, root_index: usize, child_index: usize) -> Result<()> {
        for parent_index in self.parents_of(child_index) {
            if *parent_index == root_index {
                bail!("Circular dep loop"); // TODO: keep track of loop to print better error message
            }

            self.rec_check(root_index, *parent_index)?;
        }

        Ok(())
    }

    pub fn mark_dep(&mut self, parent: &T, child: &T) -> Result<()> {
        let Some(parent_index) = self.get_index_of(parent) else {
            bail!("Parent {parent:?} not in graph");
        };

        let Some(child_index) = self.get_index_of(child) else {
            bail!("Child {child:?} not in graph");
        };

        let Some(children) = self.children.get_mut(parent_index) else {
            bail!("Graph not set up for parent {parent}");
        };

        children.push(child_index);

        let Some(parents) = self.parents.get_mut(child_index) else {
            bail!("Graph not setup for child {child}");
        };
        parents.push(parent_index);

        // Make sure we haven't built a circle
        // TODO: validate this with some unit tests
        self.rec_check(child_index, child_index)?;

        Ok(())
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

    fn parents_of(&self, idx: usize) -> &[usize] {
        &self.parents[idx]
    }

    fn leaf_nodes(&self) -> Vec<usize> {
        self.children
            .iter()
            .enumerate()
            .filter(|(_, children)| children.is_empty())
            .map(|(i, _)| i)
            .collect()
    }

    fn rec_up(&self, child_index: usize, subgraph: &mut Graph<T>) -> Result<()> {
        for parent_index in self.parents_of(child_index) {
            let child = &self.nodes[child_index];

            let parent = &self.nodes[*parent_index];

            debug!("Parent of {child} = {parent}");

            subgraph.add_node(parent.clone());
            subgraph.mark_dep(parent, child)?;
            self.rec_up(*parent_index, subgraph)?;
        }
        Ok(())
    }

    fn extract_subgraphs(&self) -> Result<Vec<Graph<T>>> {
        let leaf_nodes = self.leaf_nodes();

        let mut subgraphs = Vec::with_capacity(leaf_nodes.len());
        for child_index in leaf_nodes {
            let mut subgraph = Graph::new();
            let child = &self.nodes[child_index];
            subgraph.add_node(child.clone());

            self.rec_up(child_index, &mut subgraph)?;

            subgraphs.push(subgraph);
        }

        Ok(subgraphs)
    }

    fn rec_collect(
        &self,
        idx_stack: &mut Vec<usize>,
        val_stack: &mut Vec<T>,
        paths: &mut Vec<Vec<T>>,
    ) {
        let child_index = idx_stack.last().unwrap();

        let parents = self.parents_of(*child_index);

        if parents.is_empty() {
            paths.push(val_stack.clone());
            return;
        }

        for parent_index in parents {
            idx_stack.push(*parent_index);
            let parent = &self.nodes[*parent_index];
            val_stack.push(parent.clone());
            self.rec_collect(idx_stack, val_stack, paths);
            idx_stack.pop();
            val_stack.pop();
        }
    }

    fn paths_to_parents(&self, leaf_node: usize) -> Vec<Vec<T>> {
        let mut idx_stack = vec![leaf_node];
        let leaf = &self.nodes[leaf_node];
        let mut val_stack = vec![leaf.clone()];
        let mut paths = Vec::new();

        self.rec_collect(&mut idx_stack, &mut val_stack, &mut paths);

        paths
    }

    pub fn build_paths(self) -> Result<Vec<Vec<Vec<T>>>> {
        let subgraphs = &self.extract_subgraphs()?;

        let n = subgraphs.len();
        let mut paths = Vec::with_capacity(n);
        for graph in subgraphs {
            let leaves = graph.leaf_nodes();

            assert!(leaves.len() == 1, "Expected only one leaf node in subgraph");
            let leaf = leaves[0];
            paths.push(graph.paths_to_parents(leaf));
        }
        Ok(paths)
    }

    fn display_visit(
        &self,
        f: &mut std::fmt::Formatter<'_>,
        visited: &mut Vec<bool>,
        current_index: usize,
        level: usize,
    ) -> std::fmt::Result {
        let indent = "  ".repeat(level);

        for child_index in &self.children[current_index] {
            let child = &self.nodes[child_index.clone()];
            visited[*child_index] = true;
            writeln!(f, "{indent}- {child}")?;
            self.display_visit(f, visited, *child_index, level + 1)?;
        }

        Ok(())
    }

    pub fn roots<'a>(&'a self) -> Vec<&'a T> {
        let mut roots = Vec::new();
        for (i, parents) in self.parents.iter().enumerate() {
            if parents.is_empty() {
                roots.push(&self.nodes[i]);
            }
        }
        roots
    }

    pub fn dependency_list(&self) -> HashMap<T, HashSet<T>> {
        // Map of what each task *blocks* (the node's children)
        let mut dep_list = HashMap::new();
        for (index, node) in self.nodes.iter().enumerate() {
            let mut blocks = HashSet::new();
            for child_index in &self.children[index] {
                blocks.insert(self.nodes[*child_index].clone());
            }

            dep_list.insert(node.clone(), blocks);
        }

        dep_list
    }
}

impl<T> Display for Graph<T>
where
    T: Hash + Eq + PartialEq + Debug + Clone + Display,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let n = self.len();
        let mut visited = vec![false; n];

        let level = 0;

        for (i, node) in self.nodes.iter().enumerate() {
            if visited[i] {
                continue;
            }

            if self.parents[i].is_empty() {
                visited[i] = true;

                writeln!(f, "> {node}")?;
                self.display_visit(f, &mut visited, i, level + 1)?;
            }
        }

        Ok(())
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
