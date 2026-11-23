//! Windows: a SELECT's WINDOW clause, the parenthesised specification after
//! `OVER` and after `AS` in that clause, and a frame's mode, bounds and
//! `EXCLUDE` tail. Every frame reaching here is one the grammar admits
//! (`sql.ast.window-statement`), so each part writes what it holds without a
//! guard.

use super::*;
use crate::query::{FrameBound, NamedWindow};

impl QueryBuilder {
    /// Translate a WINDOW clause, ` WINDOW "a" AS ( … ), "b" AS ( … )`, its
    /// entries in the order the statement holds them, or nothing when it
    /// names no window.
    // [spec:pgorm:req:sql.render.window+8]
    pub(super) fn prepare_window_clause(&self, windows: &[NamedWindow], sql: &mut dyn SqlWriter) {
        windows.iter().fold(true, |first, entry| {
            write!(sql, "{}", if first { " WINDOW " } else { ", " }).unwrap();
            entry.name.prepare(sql.as_writer());
            write!(sql, " AS ").unwrap();
            match &entry.definition {
                WindowDefinition::Spec(window) => self.prepare_window_spec(window, sql),
                WindowDefinition::Extended(window) => self.prepare_extended_window(window, sql),
            }
            false
        });
    }

    /// Translate a [`WindowStatement`] into the parenthesized window
    /// specification PostgreSQL requires after `OVER` and after `WINDOW n AS`.
    // [spec:pgorm:req:sql.render.window+8]
    pub(super) fn prepare_window_spec(&self, window: &WindowStatement, sql: &mut dyn SqlWriter) {
        write!(sql, "( ").unwrap();
        if !window.partition_by.is_empty() {
            write!(sql, "PARTITION BY ").unwrap();
            window.partition_by.iter().fold(true, |first, expr| {
                if !first {
                    write!(sql, ", ").unwrap()
                }
                self.prepare_simple_expr(expr, sql);
                false
            });
        }
        self.prepare_window_tail(&window.order_by, window.frame.as_ref(), sql);
    }

    /// Translate an [`ExtendedWindow`] into `( "base" … )`: the base's name
    /// where a window of its own writes its PARTITION BY, which it copies. A
    /// WINDOW clause entry and an inline window after `OVER` write it alike.
    // [spec:pgorm:req:sql.render.window+8]
    pub(super) fn prepare_extended_window(&self, window: &ExtendedWindow, sql: &mut dyn SqlWriter) {
        write!(sql, "( ").unwrap();
        window.base.prepare(sql.as_writer());
        self.prepare_window_tail(&window.order_by, window.frame.as_ref(), sql);
    }

    /// The part of a window specification both kinds share: ` ORDER BY …`,
    /// the frame, and the closing ` )`.
    // [spec:pgorm:req:sql.render.window+8]
    fn prepare_window_tail(
        &self,
        order_by: &[OrderExpr],
        frame: Option<&FrameClause>,
        sql: &mut dyn SqlWriter,
    ) {
        if !order_by.is_empty() {
            write!(sql, " ORDER BY ").unwrap();
            order_by.iter().fold(true, |first, expr| {
                if !first {
                    write!(sql, ", ").unwrap()
                }
                self.prepare_order_expr(expr, sql);
                false
            });
        }
        if let Some(frame) = frame {
            self.prepare_frame_clause(frame, sql);
        }
        write!(sql, " )").unwrap();
    }

    /// Translate a [`FrameClause`] into ` <mode> <start>` or ` <mode> BETWEEN
    /// <start> AND <end>`, then ` EXCLUDE …` when an exclusion is set.
    // [spec:pgorm:req:sql.render.window+8]
    fn prepare_frame_clause(&self, frame: &FrameClause, sql: &mut dyn SqlWriter) {
        match frame.r#type {
            FrameType::Range => write!(sql, " RANGE ").unwrap(),
            FrameType::Rows => write!(sql, " ROWS ").unwrap(),
            FrameType::Groups => write!(sql, " GROUPS ").unwrap(),
        };
        if let Some(end) = &frame.end {
            write!(sql, "BETWEEN ").unwrap();
            self.prepare_frame_bound(&frame.start, sql);
            write!(sql, " AND ").unwrap();
            self.prepare_frame_bound(end, sql);
        } else {
            self.prepare_frame_bound(&frame.start, sql);
        }
        if let Some(exclusion) = frame.exclusion {
            write!(sql, " EXCLUDE {}", exclusion.keyword()).unwrap();
        }
    }

    /// Translate one frame bound. An offset renders through the expression
    /// path — a value as a placeholder or an inline literal, anything else as
    /// itself — and needs no parentheses, because the keyword after it ends it.
    // [spec:pgorm:req:sql.render.window+8] (frame bounds)
    fn prepare_frame_bound(&self, bound: &FrameBound, sql: &mut dyn SqlWriter) {
        match bound {
            FrameBound::UnboundedPreceding => write!(sql, "UNBOUNDED PRECEDING").unwrap(),
            FrameBound::Preceding(offset) => {
                self.prepare_simple_expr(offset, sql);
                write!(sql, " PRECEDING").unwrap();
            }
            FrameBound::CurrentRow => write!(sql, "CURRENT ROW").unwrap(),
            FrameBound::Following(offset) => {
                self.prepare_simple_expr(offset, sql);
                write!(sql, " FOLLOWING").unwrap();
            }
            FrameBound::UnboundedFollowing => write!(sql, "UNBOUNDED FOLLOWING").unwrap(),
        }
    }
}
