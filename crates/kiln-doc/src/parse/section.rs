use crate::ast::{Node, NodeKind};

pub(super) fn build_section_tree(blocks: Vec<Node>) -> Node {
    struct StackEntry {
        level: u8,
        section: Node,
    }

    let mut stack = vec![StackEntry {
        level: 0,
        section: Node::section(0, Vec::new()),
    }];

    for node in blocks {
        let Some(&level) = (match &node.kind {
            NodeKind::Heading { level } => Some(level),
            _ => None,
        }) else {
            stack
                .last_mut()
                .expect("stack has root")
                .section
                .children
                .push(node);
            continue;
        };
        // Pop sections until we find one with a lower level
        while stack.len() > 1 && stack.last().expect("stack non-empty").level >= level {
            let finished = stack.pop().expect("stack non-empty");
            stack
                .last_mut()
                .expect("stack has root")
                .section
                .children
                .push(finished.section);
        }
        // Create new section for this heading
        let mut new_section = Node::section(level, Vec::new());
        new_section.children.push(node);
        stack.push(StackEntry {
            level,
            section: new_section,
        });
    }

    // Pop remaining sections
    while stack.len() > 1 {
        let finished = stack.pop().expect("stack non-empty");
        stack
            .last_mut()
            .expect("stack has root")
            .section
            .children
            .push(finished.section);
    }

    stack.pop().expect("stack has root").section
}
